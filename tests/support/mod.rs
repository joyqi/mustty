// Each test crate uses only part of the helpers
#![allow(dead_code)]

// Pseudo-terminals (openpty) exist on Unix only; the test crates that use them are Unix-only too
#[cfg(unix)]
pub mod pty;

use mustty::editor::document::{FileMeta, LineEnding};

/// Metadata with LF line endings, so no assertion depends on the platform default (CRLF on Windows).
/// Tests use this rather than `FileMeta::new`; the library's own tests have the same `lf_meta`.
pub fn lf_meta(path: Option<&str>) -> FileMeta {
    FileMeta {
        path: path.map(Into::into),
        line_ending: LineEnding::Lf,
        has_bom: false,
        stamp: None,
    }
}
