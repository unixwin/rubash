use super::*;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use std::io::IsTerminal;

impl Executor {
    pub(in crate::executor) fn handle_external_file_builtins(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !self.external_file_builtins_enabled {
            return Ok(false);
        }
        let name = cmd.words[0].as_str();
        let emulated = matches!(
            name,
            "/bin/pwd"
                | "/usr/bin/pwd"
                | "/bin/printf"
                | "/usr/bin/printf"
                | "mkdir"
                | "touch"
                | "chmod"
                | "cp"
                | "rm"
                | "rmdir"
                | "cat"
                | "/bin/cat"
                | "/usr/bin/cat"
                | "sed"
                | "mkfifo"
                | "tty"
                | "/bin/tty"
                | "/usr/bin/tty"
        );
        // GNU findcmd.c:365/416 (search_for_command): a plain name resolved
        // through PATH for execution enters the hash table with
        // times_found=1. The emulated commands below stand in for that PATH
        // binary, so record the same resolution — `hash -t`/`hash -l`/
        // `BASH_CMDS` must see it (builtins9.sub: a stale `hash -p` entry
        // forgotten under checkhash is re-recorded by the next run).
        if emulated
            && !name.contains('/')
            && !name.contains('\\')
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "hashall")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_TEMP_PATH")
                .map(String::as_str)
                != Some("1")
        {
            if let Some(program) =
                crate::executor::path::find_user_command(name, &self.shell_state.env_vars)
            {
                let display = super::execution_misc::shell_display_path(
                    &program.to_string_lossy().replace('\\', "/"),
                );
                crate::builtins::hash::record_command_resolution(
                    &mut self.shell_state.env_vars,
                    name,
                    &display,
                );
            }
        }
        match cmd.words[0].as_str() {
            "/bin/pwd" | "/usr/bin/pwd" => {
                let mut pwd_cmd = cmd.clone();
                pwd_cmd.words[0] = "pwd".to_string();
                self.exit_code = self.execute_pwd(&pwd_cmd)?;
                Ok(true)
            }
            "/bin/printf" | "/usr/bin/printf" => {
                let mut printf_cmd = cmd.clone();
                printf_cmd.words[0] = "printf".to_string();
                self.exit_code = self.execute_printf(&printf_cmd)?;
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
        let mut failed = false;
        for word in &cmd.words[1..] {
            let expanded = self.expand_word(word);
            if !no_more_flags && !mode_value_pending && expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if !no_more_flags && !mode_value_pending && expanded.starts_with('-') && expanded != "-"
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
            // coreutils mkdir.c: a failed operand reports
            // `mkdir: cannot create directory 'NAME': <strerror>' and the
            // loop continues with the remaining operands, leaving status 1.
            // The wrapper is the cosmetic part rubash#280 fixes:
            // propagating the io::Error with `?` printed a bare
            // `<script>: line N: Invalid argument` with no mkdir context
            // (an NTFS-invalid wildcard name such as `x*s' triggers it on
            // Windows while ext4 accepts it).
            if let Err(error) =
                fs::create_dir_all(shell_path_to_windows(&expanded, &self.shell_state.env_vars))
            {
                eprintln!(
                    "{}mkdir: cannot create directory '{}': {}",
                    self.diagnostic_prefix(),
                    expanded,
                    crate::posix_errors::message(&error)
                );
                failed = true;
            }
        }
        self.exit_code = i32::from(failed);
        Ok(true)
    }

    fn external_touch(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // GNU coreutils touch.c do_touch: for an EXISTING operand,
        // utimensat(AT_FDCWD, file, NULL, 0) stamps atime and mtime with
        // the current time — the file is opened O_WRONLY only when it has
        // to be created, and the content is never truncated or rewritten.
        // The previous File::create here truncated every existing operand
        // to zero bytes. A missing operand is created empty (unless
        // -c/--no-create) and then stamped. Every operand is processed
        // independently: a failure prints `touch: cannot touch 'FILE': ...`
        // and the loop continues, leaving exit status 1 (touch.c do_touch
        // loop). Timestamp-bearing options (-t TIME, -d TIME, -r FILE)
        // consume their argument; their clock is approximated as "now" —
        // the invariants this emulation preserves are non-truncation,
        // creation, -c, and the per-operand error loop.
        let mut failed = false;
        let mut create_missing = true;
        let mut stamp_accessed = true;
        let mut stamp_modified = true;
        let mut operands: Vec<String> = Vec::new();
        let mut no_more_options = false;
        let mut index = 1;
        while let Some(word) = cmd.words.get(index) {
            index += 1;
            let expanded = self.expand_word(word);
            if no_more_options || !expanded.starts_with('-') || expanded == "-" {
                operands.push(expanded);
                continue;
            }
            if expanded == "--" {
                no_more_options = true;
                continue;
            }
            if let Some(long) = expanded.strip_prefix("--") {
                // Long options that take a value: consume it.
                if matches!(long, "date" | "reference" | "time") {
                    index += 1;
                    continue;
                }
                if long == "no-create" {
                    create_missing = false;
                } else {
                    failed = true;
                    eprintln!(
                        "{}touch: unrecognized option '--{}'",
                        self.diagnostic_prefix(),
                        long
                    );
                }
                continue;
            }
            let shorts = &expanded[1..];
            if let Some(bad) = shorts
                .chars()
                .find(|flag| !matches!(flag, 'c' | 'a' | 'm' | 'h' | 't' | 'd' | 'r'))
            {
                failed = true;
                eprintln!(
                    "{}touch: invalid option -- '{}'",
                    self.diagnostic_prefix(),
                    bad
                );
                continue;
            }
            for flag in shorts.chars() {
                match flag {
                    'c' => create_missing = false,
                    'a' => stamp_modified = false,
                    'm' => stamp_accessed = false,
                    // -t/-d/-r take a value: consume the next word.
                    't' | 'd' | 'r' => index += 1,
                    _ => {}
                }
            }
        }
        for expanded in operands {
            let target = shell_path_to_windows(&expanded, &self.shell_state.env_vars);
            let stamped = File::options().write(true).open(&target).and_then(|file| {
                let now = SystemTime::now();
                let mut times = std::fs::FileTimes::new();
                if stamp_accessed {
                    times = times.set_accessed(now);
                }
                if stamp_modified {
                    times = times.set_modified(now);
                }
                file.set_times(times)
            });
            let outcome = match stamped {
                Ok(()) => Ok(()),
                // -c on a missing operand: GNU touch skips it silently
                // (touch.c: no_create + ENOENT -> no error, no create).
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create_missing => {
                    Ok(())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::create(&target)
                    .and_then(|file| {
                        let now = SystemTime::now();
                        file.set_times(
                            std::fs::FileTimes::new()
                                .set_accessed(now)
                                .set_modified(now),
                        )
                    }),
                Err(error) => Err(error),
            };
            if let Err(error) = outcome {
                eprintln!(
                    "{}touch: cannot touch '{}': {}",
                    self.diagnostic_prefix(),
                    expanded,
                    crate::posix_errors::message(&error)
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
                    | "no-dereference" | "no-preserve" | "suffix" | "context" => {}
                    "target-directory" | "target-dir" => {
                        target_dir =
                            value.or_else(|| words.next().map(|next| self.expand_word(next)));
                    }
                    "help" => {
                        let _ = writeln!(stderr, "{}", cp_usage_text());
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                    other => {
                        let _ = writeln!(stderr, "{}cp: unknown option '--{}'", prefix, other);
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
                        let _ = writeln!(stderr, "{}cp: invalid option -- '{}'", prefix, other);
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
        let destination = shell_path_to_windows(destination_word, &self.shell_state.env_vars);

        // GNU null-device semantics: coreutils copy.c opens the destination
        // and the write goes nowhere, so `cp FILE /dev/null` succeeds without
        // creating anything. Windows has no device CopyFileExW can stat, so
        // model both directions explicitly.
        if effective.len() == 2 && crate::executor::path::is_shell_null_device(destination_word) {
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
                        prefix,
                        target_path.display(),
                        crate::posix_errors::message(&error)
                    );
                }
                continue;
            }
            let source = shell_path_to_windows(source_word, &self.shell_state.env_vars);

            // GNU coreutils cp.c copy_internal: an operand whose final
            // component is `.` (`src/.`, `./`, `.`) names the directory's
            // CONTENTS — dotfiles included — not the directory itself.
            // With an existing directory destination the children land
            // directly inside it instead of under a `dst/src` subdir.
            let source_names_contents = {
                let trimmed = source_word.replace('\\', "/");
                let trimmed = trimmed.trim_end_matches('/');
                trimmed == "." || trimmed.ends_with("/.")
            };
            let mut copy_units: Vec<(PathBuf, String)> = Vec::new();
            if source_names_contents {
                if !source.is_dir() {
                    // stat(src/.) fails with ENOTDIR when src exists but is
                    // not a directory, ENOENT when src itself is missing.
                    let detail = if source.exists() {
                        "Not a directory"
                    } else {
                        "No such file or directory"
                    };
                    let _ = writeln!(
                        stderr,
                        "{}cp: cannot stat '{}': {}",
                        prefix, source_word, detail
                    );
                    continue;
                }
                if !recursive {
                    let _ = writeln!(
                        stderr,
                        "{}cp: -r not specified; omitting directory '{}'",
                        prefix, source_word
                    );
                    continue;
                }
                if destination.exists() && !destination.is_dir() {
                    let _ = writeln!(
                        stderr,
                        "{}cp: cannot overwrite non-directory '{}' with directory '{}'",
                        prefix, destination_word, source_word
                    );
                    continue;
                }
                if destination.is_dir() {
                    let mut entries: Vec<_> = match fs::read_dir(&source) {
                        Ok(read_dir) => read_dir.filter_map(Result::ok).collect(),
                        Err(error) => {
                            let _ = writeln!(
                                stderr,
                                "{}cp: cannot access '{}': {}",
                                prefix,
                                source_word,
                                crate::posix_errors::message(&error)
                            );
                            status = 1;
                            continue;
                        }
                    };
                    entries.sort_by_key(|entry| entry.file_name());
                    for entry in entries {
                        let name = entry.file_name().to_string_lossy().to_string();
                        copy_units.push((
                            entry.path(),
                            format!("{}/{}", source_word.trim_end_matches('/'), name),
                        ));
                    }
                } else {
                    copy_units.push((source.clone(), source_word.clone()));
                }
            } else {
                copy_units.push((source.clone(), source_word.clone()));
            }

            for (source, source_word) in copy_units {
                // `/dev/std*`, `/dev/fd/N` sources name a descriptor, not a
                // filesystem path: GNU copies the fd's content into a
                // regular file (cp.c copy_internal reads through the dup'd
                // descriptor), so serve the endpoint's remaining bytes.
                if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&source_word) {
                    match self.dev_fd_operand_bytes(fd) {
                        Some(bytes) => {
                            let (target, target_display) = if destination.is_dir() {
                                let leaf = source_word
                                    .replace('\\', "/")
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&source_word)
                                    .to_string();
                                (
                                    destination.join(&leaf),
                                    format!("{}/{}", destination_word.trim_end_matches('/'), leaf),
                                )
                            } else {
                                (destination.clone(), destination_word.clone())
                            };
                            match fs::write(&target, &bytes) {
                                Ok(()) => {
                                    if verbose {
                                        let _ = writeln!(
                                            stdout,
                                            "{} -> {}",
                                            cp_quoted_name(&source_word),
                                            cp_quoted_name(&target_display)
                                        );
                                    }
                                }
                                Err(error) => {
                                    let _ = writeln!(
                                        stderr,
                                        "{}cp: cannot create '{}': {}",
                                        prefix,
                                        target.display(),
                                        crate::posix_errors::message(&error)
                                    );
                                    status = 1;
                                }
                            }
                        }
                        None => {
                            let _ = writeln!(
                                stderr,
                                "{}cp: cannot stat '{}': No such file or directory",
                                prefix, source_word
                            );
                            status = 1;
                        }
                    }
                    continue;
                }
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
                    // `cp -rv src/. dst` names children 'dst/./name' — the
                    // `/.` join is part of GNU's verbose target spelling.
                    let separator = if source_names_contents { "/./" } else { "/" };
                    (
                        destination.join(name),
                        format!(
                            "{}{}{}",
                            destination_word.trim_end_matches('/'),
                            separator,
                            leaf
                        ),
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
                                cp_quoted_name(&source_word),
                                cp_quoted_name(&target_display)
                            );
                        }
                    }
                    Err(error) => {
                        let _ = writeln!(
                            stderr,
                            "{}cp: cannot create '{}': {}",
                            prefix,
                            target.display(),
                            crate::posix_errors::message(&error)
                        );
                        status = 1;
                    }
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
            let target = shell_path_to_windows(&expanded, &self.shell_state.env_vars);
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
                &self.shell_state.env_vars,
            ));
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_cat(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // rubash#415: GNU cat parses its whole option surface (getopt_long
        // with argv permutation) before opening anything — a usage error
        // reports `cat: invalid option -- 'Z'' + the Try line and exits 1
        // with NO output, even when operands exist (`cat f -Z', WSL 9.4
        // probe perm.sh). --help/--version print the real binary's own
        // text, so they fall through to the PATH subprocess.
        let parsed = match parse_cat_argv(cmd) {
            CatParsed::HelpOrVersion => return Ok(false),
            CatParsed::Usage(message) => {
                let mut stderr = Vec::new();
                stderr.extend_from_slice(message.as_bytes());
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 1;
                return Ok(true);
            }
            CatParsed::Options { options, operands } => (options, operands),
        };
        let (options, operands) = parsed;
        let filter = |data: &[u8]| -> Vec<u8> { cat_format(data, &options) };
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                if let Some(fd) = redirect_target_fd(&target) {
                    if let Some(FdReadEndpoint::CoprocStdout { fd: pipe, .. }) =
                        self.fd_table.read_endpoint(fd)
                    {
                        // Drain the coproc's stdout pipe through the slot's
                        // real HANDLE (BROKEN_PIPE maps to EOF in read_some).
                        let mut input = Vec::new();
                        loop {
                            match crate::fd::read_some(pipe.handle, 8192) {
                                Ok(buf) if buf.is_empty() => break,
                                Ok(buf) => input.extend_from_slice(&buf),
                                Err(_) => break,
                            }
                        }
                        self.fd_table.close_input(fd);
                        self.write_cat_output(cmd, &filter(&input))?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                }
            }
        }

        if cmd.heredoc.is_some() {
            let input = self.stdin_string_for_command_mut(cmd).unwrap_or_default();
            let output =
                filter(&crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input));
            if let Some(redirect) = &cmd.append {
                let target = self.expand_redirect_target(redirect);
                // /dev/std*, /dev/fd/N aliases resolve as a dup of the
                // aliased fd (write_dev_stdio_redirect_output) — reopening
                // the alias as a literal path hit CONOUT$ and failed with
                // Permission denied under a redirected stderr (rubash#216).
                if let Some(result) = self.write_dev_stdio_redirect_output(&target, &output) {
                    result?;
                    self.exit_code = 0;
                    return Ok(true);
                }
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                file.write_all(&output)?;
                self.exit_code = 0;
                return Ok(true);
            }

            if let Some(redirect) = &cmd.redirect_out {
                let target = self.expand_redirect_target(redirect);
                if let Some(result) = self.write_dev_stdio_redirect_output(&target, &output) {
                    result?;
                    self.exit_code = 0;
                    return Ok(true);
                }
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&output)?;
                self.exit_code = 0;
                return Ok(true);
            }
        }

        // GNU cat: when file operands are present, read from them. Only fall
        // through to stdin when there are no file operands. This matters after
        // `exec 0</dev/null` which stores empty bytes in fd 0: without this
        // ordering, `cat file` would see the empty stdin and return early
        // instead of reading the file operand.
        if !operands.is_empty() {
            let mut output = Vec::new();
            // GNU cat.c: operand failures (missing file, input==output)
            // print a diagnostic and set exit 1 but do NOT stop the loop —
            // later operands are still processed.
            let mut status = 0;
            // A `-' operand is stdin at that position; the first one
            // drains the source, later ones see EOF (GNU cat.c reads
            // stdin once — shared file offset).
            let mut stdin_remaining: Option<Vec<u8>> = None;
            for word in operands {
                let target = self.expand_word(word);
                if target == "-" {
                    if stdin_remaining.is_none() {
                        // rubash#415 (CI coproc.tests hang): GNU cat streams
                        // stdin as chunks arrive (cat.c byte-copy loop); a
                        // buffered drain deadlocks interactive producers —
                        // coproc.tests runs `coproc { cat - ; }`, writes,
                        // reads the echo back, and closes the writer much
                        // later. With identity options and the REAL process
                        // stdin, flush what earlier operands collected and
                        // stream; in-memory sources and formatting keep the
                        // buffered drain (bounded, no cross-process wait).
                        if options.identity() && self.cat_stdin_is_real_process_stdin(cmd) {
                            if !output.is_empty() {
                                self.write_cat_output(cmd, &filter(&output))?;
                                output.clear();
                            }
                            self.stream_cat_stdin_operand(cmd)?;
                            stdin_remaining = Some(Vec::new());
                        } else {
                            stdin_remaining = Some(self.cat_stdin_operand_bytes(cmd));
                        }
                    }
                    if let Some(bytes) = stdin_remaining.take() {
                        output.extend(bytes);
                    }
                    continue;
                }
                // `/dev/std*`, `/dev/fd/N`, `/proc/self/fd/N`: GNU opens a
                // dup of the descriptor — the in-process equivalent reads
                // the fd endpoint's remaining bytes (subst.c shared
                // offset).
                if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&target) {
                    match self.dev_fd_operand_bytes_for_command(cmd, fd) {
                        Some(read) if read.stdout_file && !read.bytes.is_empty() => {
                            // GNU cat.c: input resolves to the file fd 1
                            // writes to — report and skip it.
                            let mut stderr = Vec::new();
                            writeln!(
                                &mut stderr,
                                "{}cat: {}: input file is output file",
                                self.diagnostic_prefix(),
                                target
                            )?;
                            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                            status = 1;
                            continue;
                        }
                        Some(read) => output.extend(read.bytes),
                        None => {
                            let mut stderr = Vec::new();
                            writeln!(
                                &mut stderr,
                                "{}cat: {}: No such file or directory",
                                self.diagnostic_prefix(),
                                target
                            )?;
                            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                            status = 1;
                        }
                    }
                    continue;
                }
                // Q11 /proc P1 (docs/proc-vfs-plan.md hook B2): synthetic
                // files are served before the filesystem.
                if let Some(bytes) = crate::proc_vfs::proc_file_content(&target) {
                    output.extend(bytes);
                    continue;
                }
                let win = shell_path_to_windows(&target, &self.shell_state.env_vars);
                // `<(cmd)` carrier path: the word names a draining stream —
                // serve the shared remainder (subst.c:7143).
                let read = match self.procsub_stream_take(&win) {
                    Some(bytes) => Ok(bytes),
                    None => fs::read(&win),
                };
                match read {
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
                        status = 1;
                    }
                }
            }
            self.write_cat_output(cmd, &filter(&output))?;
            self.exit_code = status;
            return Ok(true);
        }

        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            self.write_cat_output(
                cmd,
                &filter(&crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input)),
            )?;
            self.exit_code = 0;
            return Ok(true);
        }

        // No operands: plain stdin shapes (the operand loop above already
        // returned). This block keeps the original cascade order.
        {
            if let Some(input) = self.read_function_stdin('\0', None, false) {
                self.write_cat_output(
                    cmd,
                    &filter(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
                    ),
                )?;
                self.exit_code = 0;
                return Ok(true);
            }
            if cmd.redirect_in.is_none() && cmd.heredoc.is_none() && cmd.here_string.is_none() {
                // fd 0 may carry a real/virtual endpoint from a compound
                // redirect (`{ cat; } <&3`) — GNU reads fd 0 directly.
                if !matches!(
                    self.fd_table.read_endpoint(0),
                    None | Some(FdReadEndpoint::InheritedProcessStdin)
                ) {
                    if let Some(bytes) = self.fd_table.read_all_bytes(0) {
                        self.write_cat_output(cmd, &filter(&bytes))?;
                        self.exit_code = 0;
                        return Ok(true);
                    }
                }
            }
            if cmd.redirect_in.is_none()
                && cmd.heredoc.is_none()
                && cmd.here_string.is_none()
                && self
                    .shell_state
                    .env_vars
                    .get(INHERIT_PROCESS_STDIN)
                    .map(String::as_str)
                    == Some("1")
            {
                return self.stream_inherited_cat(cmd, &options);
            }
            return Ok(false);
        }
    }

    /// The `-' operand's bytes: the command's own stdin source (heredoc /
    /// captured stdin / fd 0 endpoint), drained once. Best-effort for the
    /// common shapes — a live inherited pipe is read to EOF.
    fn cat_stdin_operand_bytes(&mut self, cmd: &CommandNode) -> Vec<u8> {
        use std::io::Read;
        if let Some(input) = self.stdin_string_for_command_mut(cmd) {
            return crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input);
        }
        if !matches!(
            self.fd_table.read_endpoint(0),
            None | Some(FdReadEndpoint::InheritedProcessStdin)
        ) {
            if let Some(bytes) = self.fd_table.read_all_bytes(0) {
                return bytes;
            }
        }
        if cmd.redirect_in.is_none() && cmd.heredoc.is_none() && cmd.here_string.is_none() {
            let mut stdin = std::io::stdin().lock();
            let mut bytes = Vec::new();
            let _ = stdin.read_to_end(&mut bytes);
            return bytes;
        }
        Vec::new()
    }

    /// True when the command's stdin is the real process stdin — the only
    /// `cat -' source whose buffered drain can block on a live cross-process
    /// writer (coproc pipes). Mirrors the last-resort arm of
    /// cat_stdin_operand_bytes.
    fn cat_stdin_is_real_process_stdin(&self, cmd: &CommandNode) -> bool {
        if cmd.redirect_in.is_some() || cmd.heredoc.is_some() || cmd.here_string.is_some() {
            return false;
        }
        if self.stdin_string_for_command(cmd).is_some() {
            return false;
        }
        matches!(
            self.fd_table.read_endpoint(0),
            None | Some(FdReadEndpoint::InheritedProcessStdin)
        )
    }

    /// The `-' operand's streaming copy: read a chunk, emit it, repeat —
    /// GNU cat.c's byte-copy loop. Identity options only (formatting needs
    /// whole-stream state).
    fn stream_cat_stdin_operand(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        use std::io::Read;
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = stdin.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let data = buffer[..count].to_vec();
            self.write_cat_output(cmd, &data)?;
        }
        Ok(())
    }
    fn stream_inherited_cat(
        &mut self,
        cmd: &CommandNode,
        options: &CatOptions,
    ) -> Result<bool, ExecuteError> {
        use std::io::Read;

        if !options.identity() {
            // The line filters (numbering, squeeze, $-ends) carry state
            // across the whole stream, so a formatting invocation reads to
            // EOF first (GNU streams; the in-process emulation buffers —
            // bounded by the input, like every other emulated reader).
            let mut stdin = std::io::stdin().lock();
            let mut input = Vec::new();
            stdin.read_to_end(&mut input)?;
            self.write_cat_output(cmd, &cat_format(&input, options))?;
            self.exit_code = 0;
            return Ok(true);
        }
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = stdin.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let data = buffer[..count].to_vec();
            self.write_cat_output(cmd, &data)?;
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_sed(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // cmd.words are the already-expanded argv (the simple-command
        // executor expanded each word once, execute_cmd.c
        // execute_disk_command -> expand_words). Re-running expand_word here
        // would be a second expansion pass: a quoted argument whose literal
        // text contains ` or $ (e.g. sed 's/\`x\`/y/' — single-quoted data,
        // parse.y:5416 read_token_word shellquote branch) gets its decoded
        // backticks re-scanned as live command substitution and executed
        // (rubash#177). Use the argv as-is.
        let args = cmd.words[1..].to_vec();
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
        // Windows has no POSIX fifo object; creating a regular file would
        // silently change semantics (a fifo's blocking open/read behavior
        // would become an instant regular-file EOF). GNU coreutils mkfifo
        // reports per-operand failures and exits 1 — match that honestly:
        // `mkfifo: cannot create fifo 'NAME': Operation not supported`.
        // -m MODE consumes a value; other options are accepted and ignored.
        let mut mode_value_pending = false;
        let mut no_more_flags = false;
        let mut failed = false;
        for word in &cmd.words[1..] {
            let expanded = self.expand_word(word);
            if mode_value_pending {
                mode_value_pending = false;
                continue;
            }
            if !no_more_flags && expanded == "--" {
                no_more_flags = true;
                continue;
            }
            if !no_more_flags && expanded.starts_with('-') && expanded != "-" {
                if expanded == "-m" {
                    mode_value_pending = true;
                }
                continue;
            }
            let mut stderr = Vec::new();
            let _ = writeln!(
                &mut stderr,
                "mkfifo: cannot create fifo '{expanded}': Operation not supported"
            );
            self.write_default_stderr(&stderr)?;
            failed = true;
        }
        self.exit_code = if failed { 1 } else { 0 };
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
        Ok(_) => answer
            .trim_start()
            .starts_with(|c: char| c == 'y' || c == 'Y'),
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
fn cp_copy_tree(
    source: &std::path::Path,
    target: &std::path::Path,
    depth: usize,
) -> io::Result<()> {
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
            cp_copy_tree(
                entry.path().as_path(),
                &target.join(entry.file_name()),
                depth + 1,
            )?;
        }
        return Ok(());
    }
    fs::copy(source, target)?;
    Ok(())
}

/// GNU cat -v: display control characters using `^` notation and high-bit
/// bytes with `M-` prefix (coreutils cat.c cat_main + simple_cat). TAB and
/// LF pass through unchanged unless -T/-E are also given.
pub(in crate::executor) fn cat_v_filter(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for &byte in input {
        match byte {
            0x09 | 0x0a => output.push(byte),
            0x00..=0x08 | 0x0b..=0x1f => {
                output.push(b'^');
                output.push(byte + 0x40);
            }
            0x7f => {
                output.push(b'^');
                output.push(b'?');
            }
            0x80..=0xff => {
                output.extend_from_slice(b"M-");
                let low = byte & 0x7f;
                match low {
                    0x09 | 0x0a => output.push(low),
                    0x00..=0x08 | 0x0b..=0x1f => {
                        output.push(b'^');
                        output.push(low + 0x40);
                    }
                    0x7f => {
                        output.push(b'^');
                        output.push(b'?');
                    }
                    _ => output.push(low),
                }
            }
            _ => output.push(byte),
        }
    }
    output
}

/// Check whether the cat command has -v, -A, -e, or -t flags (all imply
/// show-nonprinting in GNU coreutils cat).
pub(in crate::executor) fn cat_has_show_nonprinting(cmd: &CommandNode) -> bool {
    for word in cmd.words.iter().skip(1) {
        if word == "--show-nonprinting" || word == "--show-all" {
            return true;
        }
        if word.starts_with('-') && !word.starts_with("--") && word.len() > 1 {
            for ch in word[1..].chars() {
                if ch == 'v' || ch == 'A' || ch == 'e' || ch == 't' {
                    return true;
                }
            }
        }
    }
    false
}

/// GNU coreutils cat option model (rubash#415). Byte-for-byte behavioral
/// reference: WSL GNU coreutils 9.4 `/usr/bin/cat` probes
/// (target/i415/gnu-matrix.out, corners/corners2/perm/order2 probes,
/// 2026-10-02), which carry the observable cat.c contract:
/// * `-b' beats `-n' in EVERY argv order (`-nb', `-bn', `-n -b', `-b -n'
///   all number nonblank lines only — WSL probes, order2.sh).
/// * numbering is `%6d\t' right-aligned, grows past six digits, and does
///   NOT terminate a final line the input left unterminated.
/// * `-E'/`-A'/`-e' print `$' only where the input HAS a newline; an
///   unterminated final line gets neither `$' nor a newline.
/// * `-s' collapses runs of adjacent EMPTY lines to one (across operand
///   boundaries — the concatenated stream is one filter input).
/// * `-u' is accepted and ignored.
/// * GNU getopt permutes argv: `cat f -n' still numbers, `cat f -Z' still
///   reports the invalid option; `--' ends option parsing; a bare `-'
///   operand is stdin at that position.
#[derive(Default)]
struct CatOptions {
    /// -b/--number-nonblank: number non-empty lines only (wins over -n).
    number_nonblank: bool,
    /// -n/--number: number all lines.
    number_all: bool,
    /// -s/--squeeze-blank.
    squeeze_blank: bool,
    /// -v/--show-nonprinting (implied by -e/-t/-A).
    show_nonprinting: bool,
    /// -E/--show-ends (implied by -e/-A).
    show_ends: bool,
    /// -T/--show-tabs (implied by -t/-A).
    show_tabs: bool,
}

impl CatOptions {
    /// No transformation at all: the plain-concatenation fast path keeps
    /// its byte-identical passthrough (and the streaming stdin read).
    fn identity(&self) -> bool {
        !(self.number_nonblank
            || self.number_all
            || self.squeeze_blank
            || self.show_nonprinting
            || self.show_ends
            || self.show_tabs)
    }

    fn number_mode(&self) -> Option<CatNumbering> {
        if self.number_nonblank {
            Some(CatNumbering::Nonblank)
        } else if self.number_all {
            Some(CatNumbering::All)
        } else {
            None
        }
    }
}

enum CatNumbering {
    All,
    Nonblank,
}

/// GNU cat long-option table. Order is behavior-carrying: an ambiguous
/// abbreviation lists the possibilities in table order (`--numb' ->
/// "possibilities: '--number-nonblank' '--number'", `--show-' -> show-
/// nonprinting, show-ends, show-tabs, show-all — WSL 9.4 probes).
const CAT_LONG_OPTIONS: [(&str, &str); 9] = [
    ("number-nonblank", "b"),
    ("number", "n"),
    ("squeeze-blank", "s"),
    ("show-nonprinting", "v"),
    ("show-ends", "E"),
    ("show-tabs", "T"),
    ("show-all", "A"),
    ("help", ""),
    ("version", ""),
];

enum CatParsed<'a> {
    Options {
        options: CatOptions,
        /// Operand words in argv order (`-' operands included verbatim).
        operands: Vec<&'a String>,
    },
    /// `--help' / `--version': print the real binary's own text — not
    /// emulated, the caller falls through to the PATH subprocess.
    HelpOrVersion,
    /// Usage error: the two GNU diagnostic lines, exit status 1.
    Usage(String),
}

/// GNU getopt_long over cat's argv: short clusters (-benstuvAET), long
/// options with unambiguous-prefix matching, `--' terminator, `-'
/// operand, and argv permutation (options after operands still parse).
/// Words glued to redirection operators and redirect-target words stay
/// out of the operand list, like the previous classifier kept them.
fn parse_cat_argv(cmd: &CommandNode) -> CatParsed<'_> {
    let program = cmd.words.first().map(String::as_str).unwrap_or("cat");
    let mut options = CatOptions::default();
    let mut operands: Vec<&String> = Vec::new();
    let mut options_done = false;
    let redirect_targets = cat_redirect_targets(cmd);
    let mut skip_next = false;
    for word in cmd.words.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_cat_redirect_operator_word(word) {
            skip_next = true;
            continue;
        }
        if redirect_targets.iter().any(|target| *target == word) {
            continue;
        }
        if !options_done && word.starts_with("--") && word.len() > 2 {
            let name = &word[2..];
            // Exact match wins even when the name is also a prefix of a
            // longer option (GNU getopt_long).
            let mut matches: Vec<&str> = CAT_LONG_OPTIONS
                .iter()
                .filter(|(long, _)| *long == name)
                .map(|(long, _)| *long)
                .collect();
            if matches.is_empty() {
                matches = CAT_LONG_OPTIONS
                    .iter()
                    .filter(|(long, _)| long.starts_with(name))
                    .map(|(long, _)| *long)
                    .collect();
                if matches.len() > 1 {
                    let possibilities = matches
                        .iter()
                        .map(|long| format!("'--{long}'"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    return CatParsed::Usage(format!(
                        "{program}: option '--{name}' is ambiguous; possibilities: {possibilities}\nTry '{program} --help' for more information.\n"
                    ));
                }
            }
            let Some(long) = matches.first() else {
                return CatParsed::Usage(format!(
                    "{program}: unrecognized option '--{name}'\nTry '{program} --help' for more information.\n"
                ));
            };
            match *long {
                "number-nonblank" => options.number_nonblank = true,
                "number" => options.number_all = true,
                "squeeze-blank" => options.squeeze_blank = true,
                "show-nonprinting" => options.show_nonprinting = true,
                "show-ends" => options.show_ends = true,
                "show-tabs" => options.show_tabs = true,
                "show-all" => {
                    options.show_nonprinting = true;
                    options.show_ends = true;
                    options.show_tabs = true;
                }
                "help" | "version" => return CatParsed::HelpOrVersion,
                _ => unreachable!("table is exhaustive"),
            }
            continue;
        }
        if !options_done && word == "--" {
            options_done = true;
            continue;
        }
        if !options_done && word.starts_with('-') && word.len() > 1 {
            for ch in word[1..].chars() {
                match ch {
                    'b' => options.number_nonblank = true,
                    'n' => options.number_all = true,
                    's' => options.squeeze_blank = true,
                    'v' => options.show_nonprinting = true,
                    'E' => options.show_ends = true,
                    'T' => options.show_tabs = true,
                    'e' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                    }
                    't' => {
                        options.show_nonprinting = true;
                        options.show_tabs = true;
                    }
                    'A' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                        options.show_tabs = true;
                    }
                    'u' => {}
                    other => {
                        return CatParsed::Usage(format!(
                            "{program}: invalid option -- '{other}'\nTry '{program} --help' for more information.\n"
                        ));
                    }
                }
            }
            continue;
        }
        operands.push(word);
    }
    CatParsed::Options { options, operands }
}

/// Apply the parsed cat formatting options to one complete byte stream
/// (the concatenation of all operands / stdin — GNU's line filters see
/// one stream, so a squeeze or numbering counter carries across operand
/// boundaries). Byte semantics fixed by the WSL 9.4 probes above.
fn cat_format(input: &[u8], options: &CatOptions) -> Vec<u8> {
    if options.identity() {
        return input.to_vec();
    }
    let numbering = options.number_mode();
    let mut output = Vec::with_capacity(input.len() + input.len() / 8);
    let mut number = 0u64;
    let mut previous_blank = false;
    let mut rest = input;
    while !rest.is_empty() {
        let (line, terminated, remainder) = match rest.iter().position(|&b| b == b'\n') {
            Some(index) => (&rest[..index], true, &rest[index + 1..]),
            None => (rest, false, &[][..]),
        };
        rest = remainder;
        let blank = terminated && line.is_empty();
        if options.squeeze_blank && blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        match numbering {
            Some(CatNumbering::All) => {
                number += 1;
                let _ = write!(output, "{number:>6}\t");
            }
            Some(CatNumbering::Nonblank) if !blank => {
                number += 1;
                let _ = write!(output, "{number:>6}\t");
            }
            _ => {}
        }
        if options.show_tabs {
            let mut escaped = Vec::with_capacity(line.len());
            for &byte in line {
                if byte == b'\t' {
                    escaped.extend_from_slice(b"^I");
                } else {
                    escaped.push(byte);
                }
            }
            output.extend_from_slice(&cat_v_content(&escaped, options));
        } else {
            output.extend_from_slice(&cat_v_content(line, options));
        }
        if terminated {
            if options.show_ends {
                output.push(b'$');
            }
            output.push(b'\n');
        }
        // An unterminated final line gets neither `$' nor a newline
        // (WSL 9.4 probes: `cat -E f' / `cat -n f' with no trailing LF).
    }
    output
}

/// The show-nonprinting escape over one line's content bytes (LF cannot
/// occur; a TAB passes through literally — only -T/-A convert it, and
/// that replacement already happened before this runs).
fn cat_v_content(line: &[u8], options: &CatOptions) -> Vec<u8> {
    if options.show_nonprinting {
        cat_v_filter(line)
    } else {
        line.to_vec()
    }
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
                if arg.starts_with('-')
                    && arg.len() > 1
                    && arg[1..].chars().all(|ch| "fRvc".contains(ch))
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
            let windows =
                crate::executor::path::shell_path_to_windows(file, &self.shell_state.env_vars)
                    .to_string_lossy()
                    .to_string();
            // Resolve the host path once: unix applies the mode to the real
            // inode below, windows syncs the readonly attribute.
            let host = if std::path::Path::new(&windows).is_absolute() {
                std::path::PathBuf::from(&windows)
            } else {
                std::env::current_dir().unwrap_or_default().join(&windows)
            };
            // Base mode for symbolic clauses: unix reads the file's real
            // inode mode (coreutils chmod(1) applies `+x`/`-w` clauses to
            // the file's current mode via chmod(2)); windows keeps the
            // emulated store, which is the only mode source there.
            #[cfg(unix)]
            let base = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&host)
                    .map(|metadata| metadata.permissions().mode())
                    .unwrap_or(0o644)
            };
            #[cfg(not(unix))]
            let base = crate::builtins::test::emulated_file_mode(file, &self.shell_state.env_vars)
                .unwrap_or_else(|| self.default_emulated_mode(&windows));
            match apply_chmod_mode(base, mode) {
                Some(new_mode) => {
                    #[cfg(unix)]
                    {
                        // coreutils chmod(1) -> chmod(2): the real inode mode
                        // is the single source of truth on unix; the readonly
                        // attribute + emulated store below is the Windows
                        // port (no POSIX mode bits there).
                        match std::fs::metadata(&host) {
                            Ok(metadata) => {
                                use std::os::unix::fs::PermissionsExt;
                                let mut permissions = metadata.permissions();
                                permissions.set_mode(new_mode);
                                if std::fs::set_permissions(&host, permissions).is_err() {
                                    failures += 1;
                                }
                            }
                            Err(_) => {
                                failures += 1;
                                eprintln!(
                                    "chmod: cannot access '{}': No such file or directory",
                                    file
                                );
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        // DrvFs/WSL materializes exactly one POSIX permission
                        // bit: stripping all write bits sets the Windows readonly
                        // attribute, so `>file`/`>>file` fail EACCES like under
                        // GNU-on-WSL (redir12.sub). Sync it for every real path —
                        // relative names resolve against the drive-relative cwd.
                        match std::fs::metadata(&host) {
                            Ok(metadata) => {
                                let readonly = new_mode & 0o222 == 0;
                                let mut permissions = metadata.permissions();
                                if permissions.readonly() != readonly {
                                    permissions.set_readonly(readonly);
                                    if std::fs::set_permissions(&host, permissions).is_err() {
                                        failures += 1;
                                    }
                                }
                            }
                            Err(_) => {
                                failures += 1;
                                eprintln!(
                                    "chmod: cannot access '{}': No such file or directory",
                                    file
                                );
                            }
                        }
                        // DrvFs does not support POSIX mode bits (GNU on WSL
                        // leaves files 0777 after `chmod -x`), so only non-drive
                        // paths get the emulated mode store; drive paths keep the
                        // `test -x` existence fallback (posix2 negative -x).
                        let is_drive = windows.len() >= 2
                            && windows.as_bytes()[1] == b':'
                            && windows.as_bytes()[0].is_ascii_alphabetic();
                        if !is_drive
                            && !windows.starts_with("\\\\wsl$")
                            && !windows.starts_with("//wsl$")
                        {
                            store_emulated_file_mode(
                                &mut self.shell_state.env_vars,
                                &windows,
                                new_mode,
                            );
                        }
                    }
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
    /// Windows-port-only: unix bases symbolic clauses on the real inode mode.
    #[cfg_attr(unix, allow(dead_code))]
    fn default_emulated_mode(&self, windows: &str) -> u32 {
        let executable = std::path::Path::new(windows)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "exe" | "com" | "bat" | "cmd"
                )
            })
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
    if !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed.len() <= 4 {
        return u32::from_str_radix(trimmed, 8)
            .ok()
            .map(|value| value & 0o777);
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

/// Windows-port-only emulated mode store: unix chmod writes the real inode
/// mode instead (PermissionsExt::set_mode above).
#[cfg_attr(unix, allow(dead_code))]
fn store_emulated_file_mode(env_vars: &mut HashMap<String, String>, windows: &str, mode: u32) {
    let key = crate::builtins::test::EMULATED_FILE_MODES;
    let entries = env_vars.get(key).cloned().unwrap_or_default();
    let mut kept: Vec<String> = entries
        .split(DATA_DOLLAR)
        .filter(|entry| {
            !entry.is_empty()
                && entry
                    .rsplit_once('=')
                    .map(|(path, _)| path != windows)
                    .unwrap_or(true)
        })
        .map(str::to_string)
        .collect();
    kept.push(format!("{}={:o}", windows, mode));
    env_vars.insert(key.to_string(), kept.join(DATA_DOLLAR_STR));
}
