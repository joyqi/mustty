//! Loading and atomic saving: UTF-8 validation, BOM, line endings.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::editor::document::{DiskStamp, Document, FileMeta, LineEnding};
use crate::i18n::{t, t_args};

#[derive(Debug)]
pub enum IoError {
    NotUtf8 { offset: usize },
    IsDirectory,
    ExternallyModified,
    Io(std::io::Error),
}

// Messages follow the UI language, and thiserror's #[error] only takes literals, so this is hand-written
impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 { offset } => {
                f.write_str(&t_args("io-not-utf8", &[("offset", &offset.to_string())]))
            }
            Self::IsDirectory => f.write_str(t("io-is-directory")),
            Self::ExternallyModified => f.write_str(t("io-externally-modified")),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => e.source(),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Open a file; if it doesn't exist, return an empty document that remembers the path.
pub fn load(path: &Path) -> Result<Document, IoError> {
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => Err(IoError::IsDirectory),
        Ok(m) => {
            let mut doc = decode(&fs::read(path)?, Some(path.to_path_buf()))?;
            doc.meta.stamp = stamp_of(&m);
            Ok(doc)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(Document::parse("", FileMeta::new(Some(path.to_path_buf()))))
        }
        Err(e) => Err(e.into()),
    }
}

/// Strip the BOM, detect the line ending from the first newline, and normalize to `\n`.
pub fn decode(bytes: &[u8], path: Option<PathBuf>) -> Result<Document, IoError> {
    let has_bom = bytes.starts_with(b"\xEF\xBB\xBF");
    let bom = if has_bom { 3 } else { 0 };
    let raw = std::str::from_utf8(&bytes[bom..]).map_err(|e| IoError::NotUtf8 {
        offset: bom + e.valid_up_to(),
    })?;
    let line_ending = match raw.find('\n') {
        Some(i) if raw[..i].ends_with('\r') => LineEnding::CrLf,
        Some(_) => LineEnding::Lf,
        None => LineEnding::platform(),
    };
    let meta = FileMeta {
        path,
        line_ending,
        has_bom,
        stamp: None,
    };
    Ok(Document::parse(&raw.replace("\r\n", "\n"), meta))
}

fn stamp_of(m: &fs::Metadata) -> Option<DiskStamp> {
    Some(DiskStamp {
        mtime: m.modified().ok()?,
        len: m.len(),
    })
}

/// Current mtime and size on disk; None if the file doesn't exist.
fn disk_stamp(path: &Path) -> Option<DiskStamp> {
    stamp_of(&fs::metadata(path).ok()?)
}

/// Save atomically. With `check_disk`, give up if the mtime or size on disk differs from `doc.meta.stamp`. Returns the stamp after saving.
pub fn save(doc: &Document, path: &Path, check_disk: bool) -> Result<DiskStamp, IoError> {
    if path.is_dir() {
        return Err(IoError::IsDirectory);
    }
    if check_disk && disk_stamp(path) != doc.meta.stamp {
        return Err(IoError::ExternallyModified);
    }
    atomic_write(path, doc.serialize().as_bytes())?;
    disk_stamp(path).ok_or_else(|| std::io::Error::other(t("io-stat-after-save")).into())
}

/// If `untitled-N` is still taken up to this number, fall back to a timestamp.
pub const MAX_UNTITLED: u32 = 1000;

/// File name prefilled when saving a document without a path: `{base}.{ext}`, or if taken `{base}-2.{ext}`, `{base}-3.{ext}`, …
/// The first free one wins; if everything up to `MAX_UNTITLED` is taken, use `{base}-{stamp}.{ext}`.
/// `taken` reports whether a name is in use, so this function never touches the file system.
pub fn free_name(base: &str, ext: &str, taken: impl Fn(&str) -> bool, stamp: u64) -> String {
    (1..=MAX_UNTITLED)
        .map(|n| match n {
            1 => format!("{base}.{ext}"),
            n => format!("{base}-{n}.{ext}"),
        })
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{base}-{stamp}.{ext}"))
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

/// Write a temp file in the same directory → fsync → rename to replace atomically, keeping the original
/// file's permissions (Unix). Shared by documents and the config file.
/// If `path` is a symlink, rename replaces the link with a regular file: callers that need to keep the link `canonicalize` first.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let builder = tempfile::Builder::new();
    // Only unix needs to modify the builder; rebind it so `mut` isn't unused on Windows
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut b = builder;
        // New files match normal creation: 0666, then the umask
        b.permissions(fs::Permissions::from_mode(0o666));
        b
    };
    let mut tmp = builder.tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(tmp.path(), meta.permissions())?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_name_skips_taken_names() {
        let free = |taken: &[&str]| free_name("untitled", "md", |n| taken.contains(&n), 42);
        assert_eq!(free(&[]), "untitled.md");
        assert_eq!(free(&["untitled.md"]), "untitled-2.md");
        assert_eq!(free(&["untitled.md", "untitled-2.md"]), "untitled-3.md");
        // Only the first free one: a gap in the middle gets used
        assert_eq!(free(&["untitled.md", "untitled-3.md"]), "untitled-2.md");
        // Other file names don't matter
        assert_eq!(free(&["notes.md", "untitled.txt"]), "untitled.md");
        // All of 1..=MAX_UNTITLED taken: fall back to the timestamp
        assert_eq!(
            free_name("untitled", "md", |_| true, 1_761_700_000),
            "untitled-1761700000.md"
        );
        let last = format!("untitled-{MAX_UNTITLED}.md");
        assert_eq!(free_name("untitled", "md", |n| n != last, 7), last);
    }

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
    use proptest::prelude::*;

    fn round_trip(bytes: &[u8]) -> Vec<u8> {
        decode(bytes, None).unwrap().serialize().into_bytes()
    }

    #[test]
    fn byte_exact_round_trip() {
        let cases: &[&[u8]] = &[
            b"# T\n\nbody\n",
            b"# T\r\n\r\nbody\r\nmore\r\n",
            b"\xEF\xBB\xBF# T\r\n\r\nbody",
            b"\xEF\xBB\xBFplain\n",
            b"no trailing newline",
            b"# only title",
            b"# only title\r\n",
            b"---\r\ntitle: x\r\n---\r\n# T\r\n",
            b"lone\rcr\n",
            b"",
        ];
        for bytes in cases {
            assert_eq!(
                round_trip(bytes),
                *bytes,
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn detects_line_ending_and_rejects_non_utf8() {
        let doc = decode(b"a\r\nb\nc", None).unwrap();
        assert_eq!(doc.meta.line_ending, LineEnding::CrLf);
        // Mixed line endings are normalized to the style of the first one
        assert_eq!(doc.serialize(), "a\r\nb\r\nc");
        assert!(matches!(
            decode(b"\xEF\xBB\xBFok\xff", None),
            Err(IoError::NotUtf8 { offset: 5 })
        ));
    }

    proptest! {
        #[test]
        fn any_lf_text_round_trips(s in "(# )?[a-z中 #\\-\n]{0,40}", bom: bool) {
            let mut bytes = if bom { b"\xEF\xBB\xBF".to_vec() } else { vec![] };
            bytes.extend_from_slice(s.as_bytes());
            prop_assert_eq!(round_trip(&bytes), bytes.clone());
            let crlf = s.replace('\n', "\r\n");
            prop_assert_eq!(round_trip(crlf.as_bytes()), crlf.as_bytes());
        }
    }
}
