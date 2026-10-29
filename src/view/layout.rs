//! Three-region layout, as a pure function.

use ratatui::layout::Rect;

/// Maximum body width.
pub const MEASURE: u16 = 72;
/// Minimum side margin on narrow terminals.
pub const MIN_SIDE_MARGIN: u16 = 2;
/// Blank rows between the title and the body, and between the body and the status bar.
pub const GAP: u16 = 1;
/// Top and bottom margins each take 1 / V_PAD_DIVISOR of the terminal height.
pub const V_PAD_DIVISOR: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub title: Option<Rect>,
    pub body: Rect,
    pub status: Option<Rect>,
    pub too_small: bool,
}

pub fn compute_layout(area: Rect, measure: u16) -> Layout {
    let (wt, ht) = (area.width, area.height);
    if wt < 10 || ht == 0 {
        return Layout {
            title: None,
            body: area,
            status: None,
            too_small: true,
        };
    }

    let w = if wt >= measure + 2 * MIN_SIDE_MARGIN {
        measure
    } else if wt >= 24 {
        wt - 2 * MIN_SIDE_MARGIN
    } else {
        wt
    };
    let x = area.x + (wt - w) / 2;
    let row = |y: u16, h: u16| Rect::new(x, area.y + y, w, h);

    let (title, body, status) = if ht >= 12 {
        let v_pad = (ht / V_PAD_DIVISOR).max(1);
        let block_h = ht - 2 * v_pad;
        let body_h = block_h - 2 - 2 * GAP;
        let y0 = (ht - block_h) / 2;
        (
            Some(row(y0, 1)),
            row(y0 + 1 + GAP, body_h),
            Some(row(y0 + 1 + GAP + body_h + GAP, 1)),
        )
    } else if ht >= 5 {
        (Some(row(0, 1)), row(1, ht - 2), Some(row(ht - 1, 1)))
    } else if ht >= 2 {
        (None, row(0, ht - 1), Some(row(ht - 1, 1)))
    } else {
        (None, row(0, ht), None)
    };
    Layout {
        title,
        body,
        status,
        too_small: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(w: u16, h: u16) -> Layout {
        compute_layout(Rect::new(0, 0, w, h), MEASURE)
    }

    #[test]
    fn plan_examples() {
        let l = layout(120, 40);
        assert_eq!(l.title, Some(Rect::new(24, 5, 72, 1)));
        assert_eq!(l.body, Rect::new(24, 7, 72, 26));
        assert_eq!(l.status, Some(Rect::new(24, 34, 72, 1)));

        let l = layout(80, 24);
        assert_eq!(l.title, Some(Rect::new(4, 3, 72, 1)));
        assert_eq!(l.body, Rect::new(4, 5, 72, 14));
        assert_eq!(l.status, Some(Rect::new(4, 20, 72, 1)));

        let l = layout(60, 10);
        assert_eq!(l.title, Some(Rect::new(2, 0, 56, 1)));
        assert_eq!(l.body, Rect::new(2, 1, 56, 8));
        assert_eq!(l.status, Some(Rect::new(2, 9, 56, 1)));
    }

    #[test]
    fn horizontal_levels() {
        let cases = [
            (10, 10, 0),
            (23, 23, 0),
            (24, 20, 2),
            (75, 71, 2),
            (76, 72, 2),
            (77, 72, 2), // an odd leftover column goes to the right
            (200, 72, 64),
        ];
        for (wt, w, x) in cases {
            let l = layout(wt, 24);
            assert_eq!((l.body.width, l.body.x), (w, x), "W={wt}");
        }
    }

    #[test]
    fn vertical_levels() {
        let shape = |l: Layout| (l.title.is_some(), l.status.is_some());
        for (ht, want) in [
            (1, (false, false)),
            (2, (false, true)),
            (4, (false, true)),
            (5, (true, true)),
            (11, (true, true)),
            (12, (true, true)),
        ] {
            assert_eq!(shape(layout(80, ht)), want, "H={ht}");
        }
        assert_eq!(layout(80, 11).body, Rect::new(4, 1, 72, 9));
        assert_eq!(layout(80, 4).body, Rect::new(4, 0, 72, 3));
        assert_eq!(layout(80, 1).body, Rect::new(4, 0, 72, 1));
        // H = 12: v_pad = 1, a 6-row body, and the status bar right above the bottom margin
        let l = layout(80, 12);
        assert_eq!((l.body.y, l.body.height), (3, 6));
        assert_eq!(l.status.unwrap().y, 10);
    }

    #[test]
    fn too_small() {
        assert!(layout(8, 24).too_small);
        assert!(layout(9, 24).too_small);
        assert!(!layout(10, 24).too_small);
        assert!(layout(80, 0).too_small);
    }

    #[test]
    fn invariants_on_grid() {
        for wt in [8, 9, 10, 23, 24, 75, 76, 200] {
            for ht in [1, 2, 4, 5, 11, 12, 24, 60] {
                let area = Rect::new(0, 0, wt, ht);
                let l = compute_layout(area, MEASURE);
                if l.too_small {
                    assert!(wt < 10);
                    continue;
                }
                assert!(l.body.height >= 1, "{wt}x{ht}");
                let rects = [l.title, Some(l.body), l.status];
                for r in rects.into_iter().flatten() {
                    assert_eq!(area.intersection(r), r, "{wt}x{ht} {r:?}");
                    let (left, right) = (r.x, wt - r.right());
                    assert!(right == left || right == left + 1, "{wt}x{ht} not centered");
                }
                // Regions run top to bottom without overlapping
                let ys: Vec<_> = rects.into_iter().flatten().collect();
                for p in ys.windows(2) {
                    assert!(p[0].bottom() <= p[1].y, "{wt}x{ht}");
                }
            }
        }
    }
}
