pub mod layout;
pub mod viewport;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::style::Style;

use crate::app::App;
use crate::status::StatusModel;
use layout::{MEASURE, compute_layout};
use widgets::{BodyWidget, StatusWidget, TitleWidget};

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

    if let Some(r) = layout.title {
        frame.render_widget(
            TitleWidget {
                title: &app.doc.title,
                placeholder: &app.placeholder,
            },
            r,
        );
    }
    let top = app.viewport.top;
    frame.render_widget(
        BodyWidget {
            rope: &app.doc.body,
            wrap: &mut app.wrap,
            top,
        },
        body,
    );
    if let Some(r) = layout.status {
        let position = StatusModel::position(&app.doc.body, app.cursor.head);
        frame.render_widget(
            StatusWidget {
                file_name: &app.status.file_name,
                position: &position,
            },
            r,
        );
    }

    let pos = app.wrap.char_to_visual(&app.doc.body, app.cursor.head);
    let dy = app
        .wrap
        .rows_between(&app.doc.body, (top.line, top.row), (pos.line, pos.row));
    if (0..body.height as isize).contains(&dy) {
        // 悬挂空白或恰好满行时，光标停在最后一列
        let x = body.x + pos.col.min(body.width - 1);
        frame.set_cursor_position(Position::new(x, body.y + dy as u16));
    }
}
