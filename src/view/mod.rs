pub mod layout;
pub mod theme;
pub mod viewport;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};

use crate::app::App;
use crate::editor::Focus;
use crate::io::file::IoError;
use crate::status::StatusModel;
use layout::{MEASURE, compute_layout};
use widgets::{BodyWidget, PromptWidget, StatusWidget, TitleWidget, text_width};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = compute_layout(area, MEASURE);
    if layout.too_small {
        // 不设置光标位置 → ratatui 隐藏硬件光标
        frame.buffer_mut().set_stringn(
            area.x,
            area.y,
            "窗口太小",
            area.width as usize,
            Style::default(),
        );
        return;
    }
    let body = layout.body;
    app.sync_body(body);
    let placeholder = app.placeholder();
    let ed = &app.editor;
    let mut cursor = None;

    if let Some(r) = layout.title {
        let focused = ed.focus == Focus::Title;
        let text = ed.doc().title_text();
        let title = if text.is_empty() && !focused {
            TitleWidget {
                text: &placeholder,
                style: Modifier::DIM.into(),
                cursor_col: None,
            }
        } else {
            let before: String = text.chars().take(ed.title_cursor).collect();
            TitleWidget {
                text,
                style: Modifier::BOLD.into(),
                cursor_col: focused.then(|| text_width(&before)),
            }
        };
        if let Some(col) = title.cursor_col {
            let (pad, scroll) = title.placement(r.width);
            cursor = Some(Position::new(r.x + pad + col - scroll, r.y));
        }
        frame.render_widget(title, r);
    }

    let top = app.viewport.top;
    let focus = ed.focus;
    let caret = ed.cursor;
    let selection = ed.selection();
    let (doc, wrap, highlight) = app.editor.view();
    frame.render_widget(
        BodyWidget {
            rope: doc.body(),
            wrap,
            highlight,
            top,
            selection,
        },
        body,
    );
    if focus == Focus::Body {
        let (doc, wrap, _) = app.editor.view();
        let pos = caret.visual(doc.body(), wrap);
        let dy = wrap.rows_between(doc.body(), (top.line, top.row), (pos.line, pos.row));
        if (0..body.height as isize).contains(&dy) {
            // 写满一行的行尾、悬挂空白之后：光标放在正文右侧第一格，不与最后一个字重叠
            let x = (body.x + pos.col.min(body.width)).min(area.right() - 1);
            cursor = Some(Position::new(x, body.y + dy as u16));
        }
    }

    let ed = &app.editor;
    if let Some(r) = layout.status {
        if let Some(p) = &app.status.prompt {
            let before: String = p.input.chars().take(p.cursor).collect();
            let prompt = PromptWidget {
                label: p.label(),
                input: &p.input,
                cursor_col: text_width(&before),
            };
            cursor = Some(Position::new(r.x + prompt.cursor_x(r.width), r.y));
            frame.render_widget(prompt, r);
        } else {
            let status = &app.status;
            let (left, left_style) = match &status.message {
                Some(m) => (m.text.clone(), Style::default()),
                None if status.externally_modified => {
                    (IoError::ExternallyModified.to_string(), Style::default())
                }
                None => {
                    let name = StatusModel::file_name(ed.doc().meta.path.as_deref());
                    let mark = if ed.is_dirty() { " •" } else { "" };
                    (format!("{name}{mark}"), Modifier::DIM.into())
                }
            };
            let position = match ed.focus {
                Focus::Title => "标题".to_owned(),
                Focus::Body => StatusModel::position(ed.doc().body(), ed.cursor.head),
            };
            frame.render_widget(
                StatusWidget {
                    left: &left,
                    left_style,
                    words: &status.words_label(),
                    position: &position,
                },
                r,
            );
        }
    }

    if let Some(p) = cursor {
        frame.set_cursor_position(p);
    }
}
