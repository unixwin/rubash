//! Live-console execution-level regression suite (rubash#806, the
//! interactive read family, and the stty dump/resize family). Each
//! module drives the engine under a real ConPTY through the shared
//! python support driver; the faces skip cleanly where no console
//! host is available.

mod read_conpty;
mod stty_conpty;
