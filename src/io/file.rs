//! 加载与原子保存：UTF-8 校验、BOM、换行符。

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

// 文案按界面语言取，thiserror 的 #[error] 只收字面量，所以手写
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

/// 打开文件；不存在则返回记住路径的空文档。
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

/// 去 BOM、按第一个换行判定换行符并统一为 `\n`。
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

/// 当前磁盘上的 mtime 与大小；文件不存在为 None。
fn disk_stamp(path: &Path) -> Option<DiskStamp> {
    stamp_of(&fs::metadata(path).ok()?)
}

/// 原子保存；`check_disk` 时若磁盘上的 mtime 或大小与 `doc.meta.stamp` 不同则放弃。返回保存后的 stamp。
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

/// `untitled-N` 扫到这个编号仍被占用时，改用时间戳兜底。
pub const MAX_UNTITLED: u32 = 1000;

/// 无路径文档保存时预填的文件名：`{base}.{ext}`，被占用就 `{base}-2.{ext}`、`{base}-3.{ext}`……
/// 取第一个空闲的；到 `MAX_UNTITLED` 仍被占用时用 `{base}-{stamp}.{ext}`。
/// `taken` 判断某个名字是否已被占用，所以本函数不碰文件系统。
pub fn free_name(base: &str, ext: &str, taken: impl Fn(&str) -> bool, stamp: u64) -> String {
    (1..=MAX_UNTITLED)
        .map(|n| match n {
            1 => format!("{base}.{ext}"),
            n => format!("{base}-{n}.{ext}"),
        })
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{base}-{stamp}.{ext}"))
}

/// `dir` 里第一个空闲的 `untitled*.md`。悬空的符号链接也算占用（写入会顺着链接落到别处）。
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

/// 同目录临时文件写入 → fsync → rename 原子替换；保留原文件权限（Unix）。文档与配置文件共用。
/// `path` 若是符号链接，rename 会把链接换成普通文件：需要保留链接的调用方先 `canonicalize`。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let builder = tempfile::Builder::new();
    // 只有 unix 需要改 builder；重新绑定，免得 Windows 上 `mut` 未使用
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut b = builder;
        // 新文件与普通创建一致：0666 再经 umask
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
        // 只要第一个空闲的：中间有空位就用空位
        assert_eq!(free(&["untitled.md", "untitled-3.md"]), "untitled-2.md");
        // 与别的文件名无关
        assert_eq!(free(&["notes.md", "untitled.txt"]), "untitled.md");
        // 1..=MAX_UNTITLED 全被占用：时间戳兜底
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
        // 目录也算占用
        fs::create_dir(dir.path().join("untitled-2.md")).unwrap();
        assert_eq!(untitled_name(dir.path()), "untitled-3.md");
        // 悬空的符号链接也算占用
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
        // 混合换行规范化为首个换行的风格
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
