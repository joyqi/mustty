//! The terminal host: documents and the config file live on disk.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::editor::document::{DiskStamp, Document};
use crate::host::{ConfigStore, DocStore, HostCaps};
use crate::io::file::{self, IoError, free_name};

impl HostCaps {
    /// A terminal: OSC 52 reaches the clipboard except in macOS Terminal, and Quit exits.
    pub fn native() -> Self {
        Self {
            clipboard: std::env::var_os("TERM_PROGRAM").is_none_or(|t| t != "Apple_Terminal"),
            can_quit: true,
        }
    }
}

/// Documents as files: atomic saves, with the mtime and size as the stamp.
#[derive(Debug, Default)]
pub struct FsDocStore;

impl DocStore for FsDocStore {
    fn load(&self, path: &Path) -> Result<Document, IoError> {
        file::load(path)
    }

    fn save(
        &mut self,
        doc: &Document,
        path: &Path,
        check_disk: bool,
    ) -> Result<DiskStamp, IoError> {
        file::save(doc, path, check_disk)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn untitled_name(&self, dir: &Path) -> String {
        untitled_name(dir)
    }
}

/// The first free `untitled*.md` in `dir`. A dangling symlink counts as taken (a write would follow it somewhere else).
pub fn untitled_name(dir: &Path) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    free_name(
        "untitled",
        "md",
        |name| fs::symlink_metadata(dir.join(name)).is_ok(),
        stamp,
    )
}

/// The config file on disk. A symlinked config (dotfiles kept in git) is resolved to its target,
/// so the atomic replace rewrites the target and the link itself survives.
#[derive(Debug)]
pub struct FsConfigStore {
    path: PathBuf,
}

impl FsConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The real file behind `path`; `path` itself while nothing exists there yet.
    fn target(&self) -> io::Result<PathBuf> {
        match fs::canonicalize(&self.path) {
            Ok(p) => Ok(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(self.path.clone()),
            Err(e) => Err(e),
        }
    }
}

impl ConfigStore for FsConfigStore {
    fn read(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(self.target()?) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        file::atomic_write(&self.target()?, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_name_checks_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled.md");
        fs::write(dir.path().join("untitled.md"), "").unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled-2.md");
        // A directory counts as taken
        fs::create_dir(dir.path().join("untitled-2.md")).unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled-3.md");
        // So does a dangling symlink
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("nowhere", dir.path().join("untitled-3.md")).unwrap();
            assert_eq!(untitled_name(dir.path()), "untitled-4.md");
        }
    }
}
