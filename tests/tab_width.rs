//! 单独的测试进程：tab_width 是进程级设置，不能与其他测试共享。

use mustty::text::grapheme::{grapheme_width, set_tab_width};
use mustty::text::wrap::wrap_line;

#[test]
fn configured_tab_width_drives_wrap_and_width() {
    set_tab_width(8);
    assert_eq!(grapheme_width("\t", 0), 8);
    assert_eq!(grapheme_width("\t", 3), 5);
    assert_eq!(wrap_line("a\tb", 20)[0].width, 9);
    set_tab_width(0); // 钳制到 1
    assert_eq!(grapheme_width("\t", 5), 1);
}
