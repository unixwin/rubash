//! Live-console execution-level regression suite (rubash#806, the
//! interactive read family, and the stty dump/resize family). Each
//! module drives the engine under a real pseudo-terminal through a
//! shared python support driver (ConPTY on Windows, POSIX pty
//! elsewhere); the faces skip cleanly where no terminal host is
//! available.

mod hangup_pty;
mod read_conpty;
mod read_pty;
mod stty_conpty;
