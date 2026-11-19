use super::*;

/// rubash#3372 face 3: write one describe line from byte parts plus a
/// trailing newline. Describe lines carry the name (and, for files, the
/// resolved path) — both may name a raw byte (0x9C), which a `writeln!`
/// of the folded string would re-encode as UTF-8 (C2 9C) where GNU
/// prints the byte itself (builtins/type.def describe_command prints
/// the name and the find_user_command hit verbatim).
fn write_describe_line<W: std::io::Write>(stdout: &mut W, parts: &[&[u8]]) -> std::io::Result<()> {
    for part in parts {
        stdout.write_all(part)?;
    }
    stdout.write_all(b"\n")
}

/// rubash#3372 face 3: the byte spelling of a describe display string
/// (the resolved path, or the name itself for slash-bearing names).
/// The display is built from folded strings, so its trailing component
/// is the folded name: when the display ends with it, the dir prefix
/// keeps its UTF-8 bytes and the name takes its pre-fold byte spelling
/// (raw byte, not UTF-8 re-encoding). A display that does not end with
/// the name (no name component to re-spell) keeps its bytes as-is.
/// The dir prefix is exact unless the PATH element itself carried
/// transport — that residual keeps the folded display.
fn describe_display_bytes(display: &str, folded_name: &str, name_bytes: &[u8]) -> Vec<u8> {
    if display == folded_name {
        return name_bytes.to_vec();
    }
    if let Some(dir) = display.strip_suffix(folded_name) {
        let mut out = dir.as_bytes().to_vec();
        out.extend_from_slice(name_bytes);
        return out;
    }
    display.as_bytes().to_vec()
}

impl Executor {
    // rubash#289d: the raw-stdout `describe_name` (println! describe) was
    // removed with its only caller — describes must go through
    // describe_name_with_io so output reaches the buffered single-truth
    // path (comsub capture, redirects, fd table).

    pub(in crate::executor) fn describe_name_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        // The described name is looked up and reported as a name:
        // fold a stored E400 pair to the raw char once, here, so the
        // table consults and every report line agree (rubash#3076).
        // rubash#3372 face 3: the name's byte spelling, from the
        // pre-fold word — a raw byte 0x9C prints as the byte itself,
        // a genuine U+009C as UTF-8 (transport_name_spelling_bytes
        // keeps each char's provenance; with no transport it is the
        // folded name's UTF-8 bytes, i.e. today's output).
        let name_bytes =
            crate::executor::substitution_metadata::transport_name_spelling_bytes(name);
        let folded_name = crate::executor::substitution_metadata::fold_literal_char_escapes(name);
        let name = folded_name.as_str();
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.shell_state.aliases.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            write_describe_line(
                                stdout,
                                &[
                                    &name_bytes,
                                    b" is aliased to `",
                                    alias.value.as_bytes(),
                                    b"'",
                                ],
                            )?;
                        }
                        TypeDescribeMode::Reusable => write_describe_line(
                            stdout,
                            &[b"alias ", &name_bytes, b"='", alias.value.as_bytes(), b"'"],
                        )?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "alias")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    return Ok(true);
                }
            }

            // GNU describe_command order: keywords, posix special builtins
            // before functions, then functions, then other builtins.
            if is_shell_keyword(name) {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a shell keyword"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "keyword")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            if self.posix_mode_enabled()
                && self.is_enabled_shell_builtin_name(name)
                && super::command_registry::is_special_builtin(name)
            {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a special shell builtin"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            if !skip_functions {
                if let Some(body) = self.shell_state.functions.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            // rubash#3372 face 3 residual: the function
                            // definition prints through the AST text
                            // (still folded); only the bare-name lines
                            // below take the byte spelling.
                            self.write_function_description(name, &body.commands, stdout)?
                        }
                        TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "function")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    return Ok(true);
                }
            }

            if self.is_enabled_shell_builtin_name(name) {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a shell builtin"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                return Ok(true);
            }

            // #2036 S2 disclosure, as adjudicated by #2514: the claim
            // call below reports a live in-process claim on the Windows
            // product face only; off Windows the describe answer is the
            // GNU one (the PATH file below, or not found) — see
            // describe_emulated_claim_with_io.
            if self.describe_emulated_claim_with_io(name, mode, stdout)? {
                return Ok(true);
            }
        }

        if let Some(path) = self.command_path(name, force_path) {
            // GNU builtins/type.def describe_command (disk-file branch):
            // a slash-bearing name is reported verbatim, never absolutized
            // (absolute_program => savestring(pathname); relative slash
            // names likewise print as given). The resolved absolute path
            // stays the execution answer; only the display spelling is the
            // argument itself. A3-R58B.
            let display: &str = if name.contains('/') { name } else { &path };
            // rubash#3372 face 3: the display's byte spelling — the
            // resolved path prints its raw bytes, not the folded
            // string's UTF-8 re-encoding.
            let display_bytes = describe_display_bytes(display, name, &name_bytes);
            match mode {
                TypeDescribeMode::Verbose => {
                    if crate::builtins::hash::phash_search(&self.shell_state.env_vars, name)
                        .is_some()
                    {
                        write_describe_line(
                            stdout,
                            &[&name_bytes, b" is hashed (", &display_bytes, b")"],
                        )?;
                    } else {
                        write_describe_line(stdout, &[&name_bytes, b" is ", &display_bytes])?;
                    }
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => {
                    write_describe_line(stdout, &[&display_bytes])?
                }
                TypeDescribeMode::TypeOnly => writeln!(stdout, "file")?,
            }
            return Ok(true);
        }

        Ok(false)
    }

    pub(in crate::executor) fn describe_name_all_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        force_path: bool,
        skip_functions: bool,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        // Folded like describe_name_with_io (rubash#3076).
        // rubash#3372 face 3: the name's byte spelling (see above).
        let name_bytes =
            crate::executor::substitution_metadata::transport_name_spelling_bytes(name);
        let folded_name = crate::executor::substitution_metadata::fold_literal_char_escapes(name);
        let name = folded_name.as_str();
        let mut found = false;

        // GNU type.def describe_command: with CDESC_ALL the alias/keyword/
        // function/builtin blocks run whenever !CDESC_FORCE_PATH, regardless
        // of CDESC_PATH_ONLY. Under `type -ap` they print nothing (the
        // PATH_ONLY arm is silent) but still set found=1, so the overall
        // exit status stays 0 even when no disk file matches.
        if !force_path {
            if self.alias_expansion_enabled() {
                if let Some(alias) = self.shell_state.aliases.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            write_describe_line(
                                stdout,
                                &[
                                    &name_bytes,
                                    b" is aliased to `",
                                    alias.value.as_bytes(),
                                    b"'",
                                ],
                            )?;
                        }
                        TypeDescribeMode::Reusable => write_describe_line(
                            stdout,
                            &[b"alias ", &name_bytes, b"='", alias.value.as_bytes(), b"'"],
                        )?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "alias")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    found = true;
                }
            }

            // type -a (all != 0): describe_command keeps scanning after each
            // match. Order: keyword, posix special builtin (which then
            // suppresses the duplicate plain-builtin block via skipbuiltin),
            // function, builtin.
            let mut skip_builtin = false;
            if is_shell_keyword(name) {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a shell keyword"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "keyword")?,
                    TypeDescribeMode::PathOnly => {}
                }
                found = true;
            }

            if self.posix_mode_enabled()
                && self.is_enabled_shell_builtin_name(name)
                && super::command_registry::is_special_builtin(name)
            {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a special shell builtin"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                skip_builtin = true;
                found = true;
            }

            if !skip_functions {
                if let Some(body) = self.shell_state.functions.get(name) {
                    match mode {
                        TypeDescribeMode::Verbose => {
                            // rubash#3372 face 3 residual: the function
                            // definition prints through the AST text
                            // (still folded); only the bare-name lines
                            // below take the byte spelling.
                            self.write_function_description(name, &body.commands, stdout)?
                        }
                        TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                        TypeDescribeMode::TypeOnly => writeln!(stdout, "function")?,
                        TypeDescribeMode::PathOnly => {}
                    }
                    found = true;
                }
            }

            if !skip_builtin && self.is_enabled_shell_builtin_name(name) {
                match mode {
                    TypeDescribeMode::Verbose => {
                        write_describe_line(stdout, &[&name_bytes, b" is a shell builtin"])?
                    }
                    TypeDescribeMode::Reusable => write_describe_line(stdout, &[&name_bytes])?,
                    TypeDescribeMode::TypeOnly => writeln!(stdout, "builtin")?,
                    TypeDescribeMode::PathOnly => {}
                }
                found = true;
            }

            // #2036 S2 disclosure as adjudicated by #2514 (see
            // describe_emulated_claim_with_io): on the Windows face the
            // claim line takes the builtin line's position and the PATH
            // enumeration below still runs, as it does for builtins;
            // off Windows there is no claim line and the enumeration is
            // the whole file answer, exactly as GNU's describe_command
            // prints it.
            if self.describe_emulated_claim_with_io(name, mode, stdout)? {
                found = true;
            }
        }

        for path in self.command_paths(name, force_path) {
            // Same verbatim rule as describe_name_with_io (A3-R58B):
            // slash-bearing names display as given, not absolutized.
            let display: &str = if name.contains('/') { name } else { &path };
            // rubash#3372 face 3: the display's byte spelling — the
            // resolved path prints its raw bytes, not the folded
            // string's UTF-8 re-encoding.
            let display_bytes = describe_display_bytes(display, name, &name_bytes);
            match mode {
                TypeDescribeMode::Verbose => {
                    // GNU type.def: the `is hashed (...)` wording lives only
                    // in the phash_search SHORTDESC branch, which CDESC_ALL
                    // never reaches (`all == 0 || CDESC_FORCE_PATH` guard, and
                    // -P clears CDESC_SHORTDESC). `type -a` always prints the
                    // plain `name is path` form.
                    write_describe_line(stdout, &[&name_bytes, b" is ", &display_bytes])?;
                }
                TypeDescribeMode::Reusable | TypeDescribeMode::PathOnly => {
                    write_describe_line(stdout, &[&display_bytes])?
                }
                TypeDescribeMode::TypeOnly => writeln!(stdout, "file")?,
            }
            found = true;
        }

        Ok(found)
    }

    /// #2036 S2 disclosure ruling, as adjudicated for the describe
    /// faces by #2514: when `name` carries a live simple-command
    /// claim whose registry projection is `Introspection::Emulated`,
    /// print the emulation answer for `mode` and return true;
    /// otherwise print nothing and return false. The answer forms
    /// mirror the builtin forms one class up: Verbose names the
    /// replaced body ("X is a shell emulation of <path>"), TypeOnly
    /// is the single token `emulation`, Reusable (`command -v`) is
    /// the bare name — invoking the name reproduces the in-process
    /// behavior, invoking the path would not — and PathOnly is
    /// silent-but-found, like a builtin under `type -p`.
    ///
    /// The liveness test is the dispatch test itself, face by face,
    /// so the report cannot claim a body dispatch would not run: the
    /// emulation layer needs its host flag on top of the shared
    /// stand-down predicate, the fast paths consult the predicate
    /// alone, and a Windows-only row claims nothing off Windows.
    /// Liveness implies the stand-down lookup resolved, so the path
    /// the sentence names always exists.
    ///
    /// #2514 platform adjudication (2026-11-02): the disclosure is a
    /// Windows product-face behavior and is gated to it below. Off
    /// Windows a bridge-free resolution is a genuine OS PATH file —
    /// GNU `describe_command` (builtins/type.def:211) knows no
    /// emulation kind and reports that file (`command -v` prints the
    /// full path, `type -t` prints `file`), so the describe faces
    /// must do the same: reporting scripts (modernish's `command -v`
    /// absolute-path probe is the field case) read the answer as a
    /// PATH answer. On Windows the resolved body may be the product's
    /// own applet file (the WinuxCmd bundle face the S2 windows
    /// battery pins), so the disclosure stands there unchanged; the
    /// not-found face (no bridge-free resolution) is likewise
    /// unchanged on every platform — the claim already stood down.
    /// Dispatch routing is untouched: this gates only what the
    /// describe faces print.
    ///
    /// niubash#993 §4 (2026-11-16): the disclosure is retired on the
    /// Windows face too. Git Bash reports the resolved file
    /// (`type -t cat` -> `file`, `command -v cat` -> the real path,
    /// `type -p cat` -> the real path, `type`/`command -V` ->
    /// `cat is <path>`); `emulation` is a type word GNU does not have
    /// and a bare name is a function/alias answer, never a file
    /// answer. Every describe face now takes the ordinary PATH branch
    /// below; the registry projection stays for the other consumers.
    fn describe_emulated_claim_with_io<W>(
        &self,
        name: &str,
        mode: TypeDescribeMode,
        stdout: &mut W,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
    {
        let _ = (name, mode, stdout);
        Ok(false)
    }
}
