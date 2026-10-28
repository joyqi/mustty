//! 只读加载：UTF-8 校验、去 BOM、CRLF → LF（方案 §8.3）。

use std::fs;
use std::path::Path;

use crate::editor::document::Document;

#[derive(thiserror::Error, Debug)]
pub enum IoError {
    #[error("不是 UTF-8 文件（第 {offset} 字节）")]
    NotUtf8 { offset: usize },
    #[error("mustty 只编辑文件")]
    IsDirectory,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn load(path: &Path) -> Result<Document, IoError> {
    if fs::metadata(path)?.is_dir() {
        return Err(IoError::IsDirectory);
    }
    let bytes = fs::read(path)?;
    let bom = if bytes.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let raw = std::str::from_utf8(&bytes[bom..]).map_err(|e| IoError::NotUtf8 {
        offset: bom + e.valid_up_to(),
    })?;
    Ok(Document::parse(&raw.replace("\r\n", "\n")))
}
