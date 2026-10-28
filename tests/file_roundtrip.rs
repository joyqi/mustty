use std::fs;

use mustty::editor::document::LineEnding;
use mustty::io::file::{IoError, load, save};

#[test]
fn crlf_bom_file_saved_unchanged_is_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    let original =
        "\u{feff}# 标题\r\n\r\n第一段，中文。\r\nsecond line\r\n\r\n末行无换行".as_bytes();
    fs::write(&path, original).unwrap();
    let doc = load(&path).unwrap();
    assert_eq!(doc.meta.line_ending, LineEnding::CrLf);
    assert!(doc.meta.has_bom);
    save(&doc, &path, true).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    // 没有遗留临时文件
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn missing_file_is_new_document_and_created_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.md");
    let mut doc = load(&path).unwrap();
    assert_eq!(doc.meta.path.as_deref(), Some(path.as_path()));
    doc.replace(0..0, "hello\n");
    save(&doc, &path, true).unwrap();
    let expected = if cfg!(windows) {
        "hello\r\n"
    } else {
        "hello\n"
    };
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
}

#[test]
fn missing_parent_and_directory_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let doc = load(&dir.path().join("x.md")).unwrap();
    assert!(save(&doc, &dir.path().join("no/such/x.md"), false).is_err());
    assert!(matches!(load(dir.path()), Err(IoError::IsDirectory)));
    assert!(matches!(
        save(&doc, dir.path(), false),
        Err(IoError::IsDirectory)
    ));
}

#[cfg(unix)]
#[test]
fn save_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.md");
    fs::write(&path, "x").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    save(&load(&path).unwrap(), &path, true).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn external_modification_blocks_checked_save() {
    use std::time::{Duration, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e.md");
    fs::write(&path, "v1").unwrap();
    let mut doc = load(&path).unwrap();
    // 另一个程序改了文件：内容与 mtime 都变
    fs::write(&path, "external").unwrap();
    let later = SystemTime::now() + Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(matches!(
        save(&doc, &path, true),
        Err(IoError::ExternallyModified)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "external");
    // 强制保存后记录新的 mtime，之后的检查保存恢复正常
    let stamp = save(&doc, &path, false).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "v1");
    doc.meta.stamp = Some(stamp);
    doc.meta.stamp = Some(save(&doc, &path, true).unwrap());
    // mtime 被还原（粒度粗、同步盘）但大小变了：仍判定为外部修改
    let original = doc.meta.stamp.unwrap().mtime;
    fs::write(&path, "changed by sync").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original)
        .unwrap();
    assert!(matches!(
        save(&doc, &path, true),
        Err(IoError::ExternallyModified)
    ));
    doc.meta.stamp = Some(save(&doc, &path, false).unwrap());
    // 被外部删除也视为外部修改
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        save(&doc, &path, true),
        Err(IoError::ExternallyModified)
    ));
}
