use super::*;

/// Backend governing how external commands (e.g. `cat`, `cp`, `rm`, `mkdir`,
/// `touch`, `chmod`, `rmdir`, `sed`, `mkfifo`, `tty`, `tput`) are served
/// after shell builtin/function dispatch declines (rubash#4204).
///
/// Invariant: built-in in-process emulation (`EmulatedInProcess`) and any
/// future WinuxCmd FFI backend (`WinuxCmd#1487`) are mutually exclusive
/// variants of this single seam — they must never both intercept the same
/// command layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExternalCommandBackend {
    /// Use rubash's built-in in-process emulation layer (`external_file_builtins`)
    /// subject to per-command stand-down rules, falling back to OS process spawn.
    #[default]
    EmulatedInProcess,
    /// Bypass all in-process external-command emulation and always spawn
    /// the external binary on PATH.
    ExternalSpawnOnly,
}

impl ExternalCommandBackend {
    /// Convert the existing host-facing emulation toggle into the backend it selects.
    pub(crate) const fn from_emulation_enabled(enabled: bool) -> Self {
        if enabled {
            Self::EmulatedInProcess
        } else {
            Self::ExternalSpawnOnly
        }
    }

    /// Whether this backend may serve external commands through the in-process layer.
    pub(crate) const fn uses_in_process_emulation(self) -> bool {
        matches!(self, Self::EmulatedInProcess)
    }
}

impl Executor {
    pub(in crate::executor) fn external_command_backend(&self) -> ExternalCommandBackend {
        self.external_command_backend
    }

    pub(in crate::executor) fn emulated_external_backend_active(&self) -> bool {
        self.external_command_backend().uses_in_process_emulation()
    }

    pub(in crate::executor) fn try_execute_via_external_backend(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        match self.external_command_backend() {
            ExternalCommandBackend::EmulatedInProcess => self.handle_external_file_builtins(cmd),
            ExternalCommandBackend::ExternalSpawnOnly => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_backend_defaults_to_in_process_emulation() {
        let executor = Executor::new();

        assert_eq!(
            ExternalCommandBackend::default(),
            ExternalCommandBackend::EmulatedInProcess
        );
        assert_eq!(
            executor.external_command_backend(),
            ExternalCommandBackend::EmulatedInProcess
        );
    }

    #[test]
    fn external_backend_tracks_host_emulation_toggle() {
        let mut executor = Executor::new();

        executor.set_external_file_builtins_enabled(false);
        assert_eq!(
            executor.external_command_backend(),
            ExternalCommandBackend::ExternalSpawnOnly
        );

        executor.set_external_file_builtins_enabled(true);
        assert_eq!(
            executor.external_command_backend(),
            ExternalCommandBackend::EmulatedInProcess
        );
    }

    #[test]
    fn emulated_external_backend_active_tracks_backend() {
        let mut executor = Executor::new();

        assert!(executor.emulated_external_backend_active());
        assert!(ExternalCommandBackend::EmulatedInProcess.uses_in_process_emulation());
        assert!(!ExternalCommandBackend::ExternalSpawnOnly.uses_in_process_emulation());

        executor.set_external_file_builtins_enabled(false);
        assert!(!executor.emulated_external_backend_active());

        executor.set_external_file_builtins_enabled(true);
        assert!(executor.emulated_external_backend_active());
    }

    #[test]
    fn command_substitution_executor_preserves_external_backend() {
        for enabled in [true, false] {
            let mut executor = Executor::new();
            executor.set_external_file_builtins_enabled(enabled);

            let child = executor.command_substitution_executor();

            assert_eq!(
                child.external_command_backend(),
                ExternalCommandBackend::from_emulation_enabled(enabled),
                "command-substitution child lost backend for enabled={enabled}"
            );
        }
    }
}
