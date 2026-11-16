//! Small painted graphs: axes with round-number ticks, lines, bands and markers.

use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};

use crate::theme;

/// Room left for the y labels, px.
const LEFT: f32 = 58.0;
const BOTTOM: f32 = 20.0;
const TOP: f32 = 8.0;
const RIGHT: f32 = 12.0;

/// A graph area mapping data to screen.
pub struct Axes {
    /// Where the data go, inside the labels.
    pub rect: Rect,
    pub x: [f64; 2],
    pub y: [f64; 2],
    /// A row of labels along the bottom of the data area, px. The y labels keep out of it.
    bottom_row: f32,
}

impl Axes {
    /// Lay out a graph of `height` across the available width, in a well, and return it with
    /// its painter clipped to the data area.
    pub fn allocate(ui: &mut egui::Ui, height: f32, x: [f64; 2], y: [f64; 2]) -> (Axes, Painter, egui::Response) {
        let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), height), Sense::hover());
        let outer = response.rect;
        painter.rect_filled(outer, 6.0, theme::CANVAS);
        painter.rect_stroke(outer, 6.0, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
        let rect = Rect::from_min_max(
            pos2(outer.left() + LEFT, outer.top() + TOP),
            pos2(outer.right() - RIGHT, outer.bottom() - BOTTOM),
        );
        (Axes::new(rect, x, y), painter, response)
    }

    pub fn new(rect: Rect, x: [f64; 2], y: [f64; 2]) -> Self {
        Self { rect, x: widen(x), y: widen(y), bottom_row: 0.0 }
    }

    pub fn sx(&self, x: f64) -> f32 {
        self.rect.left() + ((x - self.x[0]) / (self.x[1] - self.x[0])) as f32 * self.rect.width()
    }

    pub fn sy(&self, y: f64) -> f32 {
        self.rect.bottom() - ((y - self.y[0]) / (self.y[1] - self.y[0])) as f32 * self.rect.height()
    }

    pub fn pos(&self, x: f64, y: f64) -> Pos2 {
        pos2(self.sx(x), self.sy(y))
    }

    /// Makes room for a row of [`Axes::label`]s at `y[0]`: the y range grows down by the height
    /// of a label, and the y tick labels keep out of that row, so no tick label sits beside one.
    pub fn reserve_bottom_row(&mut self, painter: &Painter) {
        let h = painter.layout_no_wrap("0".into(), FontId::proportional(11.0), theme::TEXT_DIM).size().y;
        let px = self.rect.height();
        if px > 2.0 * h {
            self.y[0] -= (self.y[1] - self.y[0]) / f64::from(px - h) * f64::from(h);
            self.bottom_row = h;
        }
    }

    /// Data x of a screen x.
    pub fn data_x(&self, sx: f32) -> f64 {
        self.x[0] + f64::from((sx - self.rect.left()) / self.rect.width()) * (self.x[1] - self.x[0])
    }

    /// Grid lines and tick labels.
    pub fn grid(&self, painter: &Painter, x_label: impl Fn(f64) -> String, y_label: impl Fn(f64) -> String) {
        let font = FontId::proportional(11.0);
        let grid = Stroke::new(1.0, theme::faded(theme::LINE, 160));
        for x in ticks(self.x, (self.rect.width() / 90.0).max(2.0) as usize) {
            let sx = self.sx(x);
            painter.line_segment([pos2(sx, self.rect.top()), pos2(sx, self.rect.bottom())], grid);
            // The end labels are nudged in so they do not run past the canvas.
            let galley = painter.layout_no_wrap(x_label(x), font.clone(), theme::TEXT_DIM);
            let half = galley.size().x / 2.0;
            let lo = self.rect.left() - LEFT + 2.0 + half;
            let cx = sx.min(self.rect.right() + RIGHT - 2.0 - half).max(lo);
            painter.galley(pos2(cx - half, self.rect.bottom() + 3.0), galley, theme::TEXT_DIM);
        }
        for y in ticks(self.y, (self.rect.height() / 40.0).max(2.0) as usize) {
            let sy = self.sy(y);
            painter.line_segment([pos2(self.rect.left(), sy), pos2(self.rect.right(), sy)], grid);
            let galley = painter.layout_no_wrap(y_label(y), font.clone(), theme::TEXT_DIM);
            let r = Align2::RIGHT_CENTER.anchor_size(pos2(self.rect.left() - 6.0, sy), galley.size());
            if self.bottom_row > 0.0 && r.bottom() > self.rect.bottom() - self.bottom_row {
                continue;
            }
            painter.galley(r.min, galley, theme::TEXT_DIM);
        }
    }

    pub fn line(&self, painter: &Painter, points: impl IntoIterator<Item = [f64; 2]>, stroke: Stroke) {
        let pts: Vec<Pos2> =
            points.into_iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| self.pos(p[0], p[1])).collect();
        if pts.len() >= 2 {
            painter.with_clip_rect(self.rect).add(Shape::line(pts, stroke));
        }
    }

    pub fn dashed(&self, painter: &Painter, a: [f64; 2], b: [f64; 2], stroke: Stroke) {
        let shapes = Shape::dashed_line(&[self.pos(a[0], a[1]), self.pos(b[0], b[1])], stroke, 6.0, 4.0);
        painter.with_clip_rect(self.rect).extend(shapes);
    }

    pub fn dots(&self, painter: &Painter, points: impl IntoIterator<Item = [f64; 2]>, radius: f32, color: Color32) {
        let p = painter.with_clip_rect(self.rect.expand(radius));
        for q in points {
            p.circle_filled(self.pos(q[0], q[1]), radius, color);
        }
    }

    pub fn vline(&self, painter: &Painter, x: f64, stroke: Stroke) {
        let sx = self.sx(x);
        if (self.rect.left()..=self.rect.right()).contains(&sx) {
            painter.line_segment([pos2(sx, self.rect.top()), pos2(sx, self.rect.bottom())], stroke);
        }
    }

    pub fn hline(&self, painter: &Painter, y: f64, stroke: Stroke) {
        let sy = self.sy(y);
        if (self.rect.top()..=self.rect.bottom()).contains(&sy) {
            painter.line_segment([pos2(self.rect.left(), sy), pos2(self.rect.right(), sy)], stroke);
        }
    }

    /// A band across the full height between two x values.
    pub fn xband(&self, painter: &Painter, x0: f64, x1: f64, fill: Color32) {
        let r = Rect::from_x_y_ranges(self.sx(x0).min(self.sx(x1))..=self.sx(x0).max(self.sx(x1)), self.rect.y_range());
        painter.rect_filled(r.intersect(self.rect), 0.0, fill);
    }

    /// A label at a data point.
    /// Nothing is drawn for a point off the graph, and a label at an edge is moved in to fit.
    pub fn label(&self, painter: &Painter, at: [f64; 2], align: Align2, text: impl ToString, color: Color32) {
        let p = self.pos(at[0], at[1]);
        if !self.rect.expand(1.0).contains(p) {
            return;
        }
        let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(11.0), color);
        let r = align.anchor_size(p, galley.size());
        let dx = (self.rect.left() - r.left()).max(0.0) + (self.rect.right() - r.right()).min(0.0);
        let dy = (self.rect.top() - r.top()).max(0.0) + (self.rect.bottom() - r.bottom()).min(0.0);
        painter.galley(r.min + vec2(dx, dy), galley, color);
    }
}

/// A range with some width, so a flat series still draws.
fn widen(r: [f64; 2]) -> [f64; 2] {
    let (lo, hi) = (r[0].min(r[1]), r[0].max(r[1]));
    if !(lo.is_finite() && hi.is_finite()) {
        return [0.0, 1.0];
    }
    if hi - lo < 1e-12 {
        let pad = if lo.abs() > 1e-12 { lo.abs() * 0.01 } else { 1.0 };
        return [lo - pad, hi + pad];
    }
    [lo, hi]
}

/// Range of `values` with a margin of `pad` of its span either side.
pub fn span(values: impl IntoIterator<Item = f64>, pad: f64) -> [f64; 2] {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in values.into_iter().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if lo > hi {
        return [0.0, 1.0];
    }
    let m = (hi - lo) * pad;
    widen([lo - m, hi + m])
}

/// Round numbers (1, 2 or 5 times a power of ten) across `r`, about `target` of them.
pub fn ticks(r: [f64; 2], target: usize) -> Vec<f64> {
    let (lo, hi) = (r[0], r[1]);
    let raw = (hi - lo) / target.max(1) as f64;
    if !(raw.is_finite() && raw > 0.0) {
        return Vec::new();
    }
    let exp = raw.log10().floor() as i32;
    // The step that gives the count nearest the target.
    let (m, exp) = [(1.0, exp), (2.0, exp), (5.0, exp), (1.0, exp + 1)]
        .into_iter()
        .min_by(|a, b| {
            let off = |(m, e): (f64, i32)| ((hi - lo) / (m * 10f64.powi(e)) - target as f64).abs();
            off(*a).total_cmp(&off(*b))
        })
        .unwrap_or((1.0, exp));
    let step = m * 10f64.powi(exp);
    // k·m / 10^−exp rather than k·step, so 0.6 comes out as 0.6.
    let at = |k: i64| if exp < 0 { k as f64 * m / 10f64.powi(-exp) } else { k as f64 * step };
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last).map(at).collect()
}

/// A number with as many decimals as the tick step needs.
pub fn tick_text(v: f64, step_hint: f64) -> String {
    let decimals = if step_hint >= 1.0 { 0 } else { (-step_hint.log10()).ceil() as usize };
    format!("{v:.decimals$}")
}

/// Seconds as m:ss or h:mm:ss.
pub fn clock(s: f64) -> String {
    let neg = s < 0.0;
    let t = s.abs().round() as u64;
    let (h, m, sec) = (t / 3600, t / 60 % 60, t % 60);
    let sign = if neg { "-" } else { "" };
    if h > 0 { format!("{sign}{h}:{m:02}:{sec:02}") } else { format!("{sign}{m}:{sec:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_round() {
        assert_eq!(ticks([0.0, 10.0], 5), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(ticks([0.13, 0.61], 4), vec![0.2, 0.3, 0.4, 0.5, 0.6]);
        assert!(ticks([1.0, 1.0], 4).is_empty());
    }

    #[test]
    fn clock_reads() {
        assert_eq!(clock(65.0), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-30.0), "-0:30");
    }
}
