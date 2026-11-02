//! Entering and restoring terminal modes; the code lives in the native host, this path stays for the binary.

pub use crate::host::native::{TerminalGuard, set_mouse_capture};
