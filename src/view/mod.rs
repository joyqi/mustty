pub mod layout;
pub mod mouse;
pub mod screen;
pub mod settings;
pub mod theme;
pub mod viewport;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};

use crate::app::App;
use crate::editor::Focus;
use crate::i18n::t;
use crate::io::file::IoError;
use crate::status::{SaveMark, StatusModel};
use crate::text::wrap::line_text;
use crate::view::layout::Layout;
use layout::compute_layout;
use screen::{ScreenMap, TitleMap};
use widgets::{BodyWidget, PromptWidget, StatusWidget, TitleWidget, text_width, title_placement};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = compute_layout(area, app.measure());
    if layout.too_small {
        app.screen = None;
        // 不设置光标位置 → ratatui 隐藏硬件光标
        frame.buffer_mut().set_stringn(
            area.x,
            area.y,
            t("status-window-too-small"),
            area.width as usize,
            Style::default(),
        );
        return;
    }
    let body = layout.body;
    app.sync_body(body);
    if app.settings.is_some() {
        render_settings(frame, app, &layout);
        return;
    }
    let placeholder = app.placeholder();
    let tab = app.config.tab_width;
    let ed = &app.editor;
    let mut cursor = None;

    // 标题：放得下时居中，否则以光标为中心水平滚动；摆放结果记进 ScreenMap
    let focused = ed.focus == Focus::Title;
    let text = ed.doc().title_text();
    let (title_text, title_style, title_col) = if text.is_empty() && !focused {
        (placeholder.as_str(), Style::from(Modifier::DIM), None)
    } else {
        let before: String = text.chars().take(ed.title_cursor).collect();
        let col = focused.then(|| text_width(&before, tab));
        (text, Style::from(Modifier::BOLD), col)
    };
    let title_map = layout.title.map(|rect| {
        let (pad, scroll) = title_placement(title_text, title_col, rect.width, tab);
        TitleMap { rect, pad, scroll }
    });
    let map = ScreenMap::new(body, &app.viewport, title_map);
    if let Some(t) = map.title {
        let widget = TitleWidget {
            text: title_text,
            style: title_style,
            pad: t.pad,
            scroll: t.scroll,
            tab_width: tab,
        };
        frame.render_widget(widget, t.rect);
        cursor = title_col.and_then(|col| map.title_screen(col));
    }

    let top = map.top;
    let focus = ed.focus;
    let caret = ed.cursor;
    let selection = app.visible_selection();
    let paragraph = (app.config.focus_mode && focus == Focus::Body).then(|| {
        let rope = ed.doc().body();
        let visible_end = (top.line + body.height as usize + 1).min(rope.len_lines());
        current_paragraph(rope, rope.char_to_line(caret.head), top.line, visible_end)
    });
    let code_color = app.config.code_color.color();
    let (doc, wrap, highlight) = app.editor.view();
    frame.render_widget(
        BodyWidget {
            rope: doc.body(),
            wrap,
            highlight,
            top,
            top_padding: map.top_padding,
            selection,
            focus: paragraph,
            code_color,
        },
        body,
    );
    if focus == Focus::Body {
        let (doc, wrap, _) = app.editor.view();
        let pos = caret.visual(doc.body(), wrap);
        cursor = map.body_screen(pos, wrap, doc.body(), area.right());
    }
    app.screen = Some(map);

    let ed = &app.editor;
    if let Some(r) = layout.status {
        if let Some(p) = &app.status.prompt {
            let before: String = p.input.chars().take(p.cursor).collect();
            let prompt = PromptWidget {
                label: p.label(),
                input: &p.input,
                cursor_col: text_width(&before, tab),
                tab_width: tab,
            };
            cursor = Some(Position::new(r.x + prompt.cursor_x(r.width), r.y));
            frame.render_widget(prompt, r);
        } else {
            let status = &app.status;
            // 消息与外部修改提示覆盖左段（连同标记）；否则是文件名 + 保存状态标记
            let (left, left_style, mark) = match &status.message {
                Some(m) => (m.text.clone(), Style::default(), None),
                None if status.externally_modified => (
                    IoError::ExternallyModified.to_string(),
                    Style::default(),
                    None,
                ),
                None => {
                    let name = StatusModel::file_name(ed.doc().meta.path.as_deref());
                    let mark = SaveMark::of(ed.is_dirty(), ed.doc().meta.stamp.is_some());
                    (name, Modifier::DIM.into(), mark)
                }
            };
            let left = match app.mode_label() {
                Some(mode) => format!("{mode} {left}"),
                None => left,
            };
            let position = match ed.focus {
                Focus::Title => t("status-title").to_owned(),
                Focus::Body => StatusModel::position(ed.doc().body(), ed.cursor.head),
            };
            frame.render_widget(
                StatusWidget {
                    left: &left,
                    left_style,
                    mark: mark.map(|m| (m.symbol(), m.style())),
                    words: &status.words_label(),
                    position: &position,
                    hint: status.hint_until.map(|_| t("status-hint")),
                    tab_width: tab,
                },
                r,
            );
        }
    }

    if let Some(p) = cursor {
        frame.set_cursor_position(p);
    }
}

/// 设置面板：标题行 `Settings`，正文区换成面板，status bar 显示操作提示；不显示光标。
fn render_settings(frame: &mut Frame, app: &mut App, layout: &Layout) {
    app.screen = None;
    let tab = app.config.tab_width;
    let (rows, _) = settings::list_rows(layout.body.height);
    let Some(panel) = app.settings.as_mut() else {
        return;
    };
    let scroll = panel.follow(rows);
    let selected = panel.selected;
    if let Some(rect) = layout.title {
        let text = t("settings-title");
        let (pad, scroll) = title_placement(text, None, rect.width, tab);
        let widget = TitleWidget {
            text,
            style: Style::from(Modifier::BOLD),
            pad,
            scroll,
            tab_width: tab,
        };
        frame.render_widget(widget, rect);
    }
    let widget = settings::SettingsWidget {
        config: &app.config,
        cli: app.cli_layer(),
        selected,
        scroll,
    };
    frame.render_widget(widget, layout.body);
    if let Some(r) = layout.status {
        // 写回失败等错误消息优先于操作提示
        let (text, style) = match &app.status.message {
            Some(m) => (m.text.as_str(), Style::default()),
            None => (t("settings-hint"), Style::from(Modifier::DIM)),
        };
        frame
            .buffer_mut()
            .set_stringn(r.x, r.y, text, r.width as usize, style);
    }
}

/// 光标所在段落（相邻非空行）的逻辑行区间，只在 `[lo, hi)` 内查找（可见范围之外无需判断）。
fn current_paragraph(
    rope: &ropey::Rope,
    line: usize,
    lo: usize,
    hi: usize,
) -> std::ops::Range<usize> {
    let blank = |l: usize| line_text(rope, l).trim().is_empty();
    if blank(line) {
        return line..line + 1;
    }
    let mut start = line;
    while start > lo && !blank(start - 1) {
        start -= 1;
    }
    let mut end = line + 1;
    while end < hi && !blank(end) {
        end += 1;
    }
    start..end
}
