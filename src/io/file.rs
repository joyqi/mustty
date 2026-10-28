//! 加载与原子保存：UTF-8 校验、BOM、换行符（方案 §8.1–§8.3）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::editor::document::{Document, FileMeta, LineEnding};

#[derive(thiserror::Error, Debug)]
pub enum IoError {
    #[error("不是 UTF-8 文件（第 {offset} 字节）")]
    NotUtf8 { offset: usize },
    #[error("mustty 只编辑文件")]
    IsDirectory,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 打开文件；不存在则返回记住路径的空文档。
pub fn load(path: &Path) -> Result<Document, IoError> {
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => Err(IoError::IsDirectory),
        Ok(_) => decode(&fs::read(path)?, Some(path.to_path_buf())),
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
    };
    Ok(Document::parse(&raw.replace("\r\n", "\n"), meta))
}

/// 同目录临时文件写入 → fsync → 原子替换；保留原文件权限（Unix）。
pub fn save(doc: &Document, path: &Path) -> Result<(), IoError> {
    if path.is_dir() {
        return Err(IoError::IsDirectory);
    }
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 新文件与普通创建一致：0666 再经 umask
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut tmp = builder.tempfile_in(dir)?;
    tmp.write_all(doc.serialize().as_bytes())?;
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
