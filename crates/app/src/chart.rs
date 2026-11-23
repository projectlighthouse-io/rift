//! A small chart: line, bar, area or scatter in the design's coral, painted directly, plus the
//! same chart as SVG/PNG for export.

use std::rc::Rc;

use chrono::Datelike;
use gpui::prelude::*;
use gpui::{
    App, Bounds, Hsla, PathBuilder, Pixels, SharedString, TextAlign, TextRun, Window, canvas, font,
    point, px, size,
};
use types::{ChartData, ChartKind, Value};

use crate::theme;

/// Series colours: coral first, then the soft tag palette.
const PALETTE: [u32; 6] = [
    theme::ACCENT,
    0x88a6dc,
    0x7bb08a,
    0xa597d8,
    0xd6a06a,
    0x72aeb3,
];
const LEFT: f32 = 44.0;
const BOTTOM: f32 = 28.0;
const TOP: f32 = 8.0;
const RIGHT: f32 = 12.0;

/// What to draw.
#[derive(Clone)]
pub(crate) struct ChartModel {
    pub(crate) kind: ChartKind,
    pub(crate) data: ChartData,
    pub(crate) show_points: bool,
}

/// 1, 2 or 5 × 10ⁿ step giving about four ticks up to `max`.
fn nice(max: f64) -> (f64, f64) {
    if max <= 0.0 {
        return (1.0, 0.25);
    }
    let raw = max / 4.0;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    ((max / step).ceil() * step, step)
}

/// `20000` → `20k`, `1200000` → `1.2M`.
pub(crate) fn short_number(v: f64) -> String {
    let abs = v.abs();
    let (n, suffix) = if abs >= 1e9 {
        (v / 1e9, "B")
    } else if abs >= 1e6 {
        (v / 1e6, "M")
    } else if abs >= 1e3 {
        (v / 1e3, "k")
    } else {
        (v, "")
    };
    let text = if (n - n.round()).abs() < 1e-9 {
        format!("{}", n.round())
    } else {
        format!("{n:.1}")
    };
    format!("{text}{suffix}")
}

fn date_of(v: &Value) -> Option<chrono::NaiveDate> {
    match v {
        Value::Date(d) => Some(*d),
        Value::Timestamp(t) => Some(t.date()),
        Value::TimestampTz(t) => Some(t.date_naive()),
        Value::Text(s) | Value::Unknown { raw: s, .. } => {
            chrono::NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
        }
        _ => None,
    }
}

/// X labels: dates show `Nov 7` then only the day while the month stays the same.
fn x_labels(xs: &[Value]) -> Vec<String> {
    let mut last_month = None;
    xs.iter()
        .map(|x| match date_of(x) {
            Some(d) if last_month == Some(d.month()) => d.day().to_string(),
            Some(d) => {
                last_month = Some(d.month());
                format!("{} {}", d.format("%b"), d.day())
            }
            None => {
                let t = x.to_string();
                if t.chars().count() > 12 {
                    format!("{}…", t.chars().take(11).collect::<String>())
                } else {
                    t
                }
            }
        })
        .collect()
}

/// `Nov 7 – Nov 20` when X holds dates.
pub(crate) fn date_range(data: &ChartData) -> Option<String> {
    let xs: Vec<chrono::NaiveDate> = data
        .series
        .iter()
        .flat_map(|s| s.points.iter().filter_map(|p| date_of(&p.x)))
        .collect();
    let (a, b) = (xs.iter().min()?, xs.iter().max()?);
    Some(format!("{} – {}", a.format("%b %-d"), b.format("%b %-d")))
}

/// Shared geometry for the canvas and the SVG.
struct Layout {
    xs: Vec<Value>,
    y_max: f64,
    y_step: f64,
}

fn layout(data: &ChartData) -> Layout {
    let longest = data.series.iter().max_by_key(|s| s.points.len());
    let xs = longest
        .map(|s| s.points.iter().map(|p| p.x.clone()).collect())
        .unwrap_or_default();
    let max = data
        .series
        .iter()
        .flat_map(|s| s.points.iter().map(|p| p.y))
        .fold(0.0_f64, f64::max);
    let (y_max, y_step) = nice(max);
    Layout { xs, y_max, y_step }
}

impl Layout {
    fn x_index(&self, x: &Value) -> Option<usize> {
        let key = x.to_string();
        self.xs.iter().position(|v| v.to_string() == key)
    }

    /// Fraction 0–1 across the plot for the i-th category.
    fn fx(&self, i: usize, bars: bool) -> f64 {
        let n = self.xs.len().max(1) as f64;
        if bars {
            (i as f64 + 0.5) / n
        } else if n <= 1.0 {
            0.5
        } else {
            i as f64 / (n - 1.0)
        }
    }
}

fn hsla(hex: u32, alpha: f32) -> Hsla {
    let mut c = theme::hsla(hex);
    c.a = alpha;
    c
}

fn paint_text(
    text: &str,
    at: gpui::Point<Pixels>,
    align: TextAlign,
    width: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let run = TextRun {
        len: text.len(),
        font: font(theme::SANS),
        color: theme::hsla(theme::FAINT),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped =
        window
            .text_system()
            .shape_line(SharedString::from(text.to_owned()), px(11.), &[run], None);
    let _ = shaped.paint(at, px(14.), align, Some(px(width)), window, cx);
}

/// The chart as a GPUI element filling its parent.
pub(crate) fn chart_element(model: ChartModel) -> impl IntoElement {
    let model = Rc::new(model);
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, cx| {
            let l = layout(&model.data);
            let bars = model.kind == ChartKind::Bar;
            // `Number` cards draw a bare sparkline: no grid, first and last X label only.
            let spark = model.kind == ChartKind::Number;
            let (left, top, bottom) = if spark {
                (0.0, 8.0, 22.0)
            } else {
                (LEFT, TOP, BOTTOM)
            };
            let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let w = f32::from(bounds.size.width) - left - if spark { 6.0 } else { RIGHT };
            let h = f32::from(bounds.size.height) - top - bottom;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let px_x = |f: f64| ox + left + (f as f32) * w;
            let px_y = |y: f64| oy + top + h - (y / l.y_max) as f32 * h;
            // Grid and Y labels.
            let mut t = 0.0;
            while !spark && t <= l.y_max + 1e-9 {
                let y = px_y(t);
                window.paint_quad(gpui::fill(
                    Bounds::new(point(px(ox + left), px(y)), size(px(w), px(1.))),
                    theme::hsla(theme::LINE),
                ));
                paint_text(
                    &short_number(t),
                    point(px(ox), px(y - 7.)),
                    TextAlign::Right,
                    left - 8.,
                    window,
                    cx,
                );
                t += l.y_step;
            }
            // X labels, about seven.
            let labels = x_labels(&l.xs);
            let every = if spark {
                labels.len().saturating_sub(1).max(1)
            } else {
                (labels.len() / 7).max(1)
            };
            for (i, label) in labels.iter().enumerate().filter(|(i, _)| i % every == 0) {
                let x = px_x(l.fx(i, bars));
                paint_text(
                    label,
                    point(px(x - 40.), px(oy + top + h + 8.)),
                    TextAlign::Center,
                    80.,
                    window,
                    cx,
                );
            }
            let series_count = model.data.series.len().max(1);
            for (s, series) in model.data.series.iter().enumerate() {
                let color = PALETTE[s % PALETTE.len()];
                let pts: Vec<(f32, f32)> = series
                    .points
                    .iter()
                    .filter_map(|p| Some((px_x(l.fx(l.x_index(&p.x)?, bars)), px_y(p.y))))
                    .collect();
                match model.kind {
                    ChartKind::Bar => {
                        let slot = w / l.xs.len().max(1) as f32;
                        let bw = (slot * 0.7 / series_count as f32).max(1.0);
                        for (x, y) in &pts {
                            let left = x - slot * 0.35 + bw * s as f32;
                            let bottom = oy + top + h;
                            window.paint_quad(
                                gpui::fill(
                                    Bounds::new(
                                        point(px(left), px(*y)),
                                        size(px(bw), px(bottom - y)),
                                    ),
                                    theme::hsla(color),
                                )
                                .corner_radii(px(2.)),
                            );
                        }
                    }
                    kind => {
                        // Lines get a faint wash underneath (the design); Area a stronger one.
                        if kind != ChartKind::Scatter && pts.len() > 1 {
                            let mut b = PathBuilder::fill();
                            b.move_to(point(px(pts[0].0), px(oy + top + h)));
                            for (x, y) in &pts {
                                b.line_to(point(px(*x), px(*y)));
                            }
                            b.line_to(point(px(pts[pts.len() - 1].0), px(oy + top + h)));
                            b.close();
                            if let Ok(path) = b.build() {
                                let alpha = if kind == ChartKind::Line { 0.08 } else { 0.14 };
                                window.paint_path(path, hsla(color, alpha));
                            }
                        }
                        if kind != ChartKind::Scatter && pts.len() > 1 {
                            let mut b = PathBuilder::stroke(px(2.));
                            b.move_to(point(px(pts[0].0), px(pts[0].1)));
                            for (x, y) in &pts[1..] {
                                b.line_to(point(px(*x), px(*y)));
                            }
                            if let Ok(path) = b.build() {
                                window.paint_path(path, theme::hsla(color));
                            }
                        }
                        if spark && let Some((x, y)) = pts.last() {
                            window.paint_quad(
                                gpui::fill(
                                    Bounds::new(
                                        point(px(x - 4.), px(y - 4.)),
                                        size(px(8.), px(8.)),
                                    ),
                                    theme::hsla(color),
                                )
                                .corner_radii(px(4.)),
                            );
                        } else if model.show_points || kind == ChartKind::Scatter {
                            for (x, y) in &pts {
                                window.paint_quad(
                                    gpui::fill(
                                        Bounds::new(
                                            point(px(x - 3.5), px(y - 3.5)),
                                            size(px(7.), px(7.)),
                                        ),
                                        theme::hsla(theme::BG),
                                    )
                                    .corner_radii(px(3.5))
                                    .border_widths(px(1.5))
                                    .border_color(theme::hsla(color)),
                                );
                            }
                        }
                    }
                }
            }
        },
    )
    .size_full()
}

/// Any card or result chart: canvas for plotted kinds, plain elements for the number and bar list.
pub(crate) fn chart_view(model: ChartModel) -> gpui::AnyElement {
    match model.kind {
        ChartKind::Number => number_element(model).into_any_element(),
        ChartKind::BarList => bar_list_element(&model.data).into_any_element(),
        _ => chart_element(model).into_any_element(),
    }
}

/// The latest value and its change from the point before it, in percent.
pub(crate) fn latest(data: &ChartData) -> Option<(f64, Option<f64>)> {
    let points = &data.series.first()?.points;
    let last = points.last()?.y;
    let previous = points.len().checked_sub(2).map(|i| points[i].y);
    Some((
        last,
        previous
            .filter(|p| *p != 0.0)
            .map(|p| (last - p) / p.abs() * 100.0),
    ))
}

fn number_text(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        crate::ui::thousands(v.round() as u64)
    } else {
        format!("{v:.2}")
    }
}

fn number_element(model: ChartModel) -> impl IntoElement {
    let (value, delta) = latest(&model.data).unwrap_or((0.0, None));
    gpui::div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            gpui::div()
                .flex()
                .items_baseline()
                .gap(px(10.))
                .child(
                    gpui::div()
                        .text_size(px(32.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(number_text(value)),
                )
                .children(delta.map(|d| {
                    gpui::div()
                        .text_size(px(12.))
                        .text_color(theme::c(if d >= 0.0 {
                            theme::NEW_TEXT
                        } else {
                            theme::DELETED_TEXT
                        }))
                        .child(format!(
                            "{}{d:.1}% vs previous",
                            if d >= 0.0 { "+" } else { "" }
                        ))
                })),
        )
        .child(gpui::div().flex_1().min_h_0().child(chart_element(model)))
}

fn bar_list_element(data: &ChartData) -> impl IntoElement {
    let mut points: Vec<_> = data
        .series
        .first()
        .map(|s| s.points.clone())
        .unwrap_or_default();
    points.sort_by(|a, b| b.y.total_cmp(&a.y));
    let max = points.iter().map(|p| p.y).fold(0.0_f64, f64::max).max(1e-9);
    gpui::div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .text_size(px(12.5))
        .children(points.into_iter().map(move |p| {
            gpui::div()
                .flex()
                .items_center()
                .gap(px(12.))
                .h(px(18.))
                .child(
                    gpui::div()
                        .w(px(96.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(p.x.to_string()),
                )
                .child(
                    gpui::div()
                        .flex_1()
                        .h(px(9.))
                        .rounded(px(2.))
                        .bg(theme::c(theme::SELECTION))
                        .child(
                            gpui::div()
                                .h_full()
                                .w(gpui::relative((p.y / max) as f32))
                                .rounded(px(2.))
                                .bg(theme::c(theme::ACCENT)),
                        ),
                )
                .child(
                    gpui::div()
                        .w(px(56.))
                        .flex()
                        .justify_end()
                        .text_color(theme::c(theme::MUTED))
                        .child(number_text(p.y)),
                )
        }))
}

/// Keeps points whose X is a date within the last `days` days; other X values are kept.
// ponytail: trims after aggregating, so the database still scans all time; push a WHERE into
// AggregateRequest if dashboards get slow.
pub(crate) fn within_days(mut data: ChartData, days: Option<i64>) -> ChartData {
    if let Some(days) = days {
        let since = chrono::Utc::now().date_naive() - chrono::Duration::days(days - 1);
        for s in &mut data.series {
            s.points
                .retain(|p| date_of(&p.x).is_none_or(|d| d >= since));
        }
    }
    data
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn esc(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The same chart as SVG, with a title line.
pub(crate) fn svg(model: &ChartModel, title: &str, width: f32, height: f32) -> String {
    let l = layout(&model.data);
    let bars = model.kind == ChartKind::Bar;
    let top = TOP + 36.0;
    let w = width - LEFT - RIGHT - 24.0;
    let h = height - top - BOTTOM - 12.0;
    let x0 = 12.0 + LEFT;
    let px_x = |f: f64| x0 + (f as f32) * w;
    let px_y = |y: f64| top + h - (y / l.y_max) as f32 * h;
    let mut out = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' font-family='Inter'>\
         <rect width='100%' height='100%' fill='#ffffff'/>\
         <text x='12' y='24' font-size='15' font-weight='600' fill='{}'>{}</text>",
        hex(theme::TEXT),
        esc(title)
    );
    let mut t = 0.0;
    while t <= l.y_max + 1e-9 {
        let y = px_y(t);
        out.push_str(&format!(
            "<line x1='{x0}' x2='{}' y1='{y}' y2='{y}' stroke='{}'/><text x='{}' y='{}' font-size='11' text-anchor='end' fill='{}'>{}</text>",
            x0 + w,
            hex(theme::LINE),
            x0 - 8.0,
            y + 4.0,
            hex(theme::FAINT),
            short_number(t)
        ));
        t += l.y_step;
    }
    let labels = x_labels(&l.xs);
    let every = (labels.len() / 7).max(1);
    for (i, label) in labels.iter().enumerate().filter(|(i, _)| i % every == 0) {
        out.push_str(&format!(
            "<text x='{}' y='{}' font-size='11' text-anchor='middle' fill='{}'>{}</text>",
            px_x(l.fx(i, bars)),
            top + h + 20.0,
            hex(theme::FAINT),
            esc(label)
        ));
    }
    let n = model.data.series.len().max(1);
    for (s, series) in model.data.series.iter().enumerate() {
        let color = hex(PALETTE[s % PALETTE.len()]);
        let pts: Vec<(f32, f32)> = series
            .points
            .iter()
            .filter_map(|p| Some((px_x(l.fx(l.x_index(&p.x)?, bars)), px_y(p.y))))
            .collect();
        let poly: Vec<String> = pts.iter().map(|(x, y)| format!("{x},{y}")).collect();
        match model.kind {
            ChartKind::Bar => {
                let slot = w / l.xs.len().max(1) as f32;
                let bw = (slot * 0.7 / n as f32).max(1.0);
                for (x, y) in &pts {
                    out.push_str(&format!(
                        "<rect x='{}' y='{y}' width='{bw}' height='{}' rx='2' fill='{color}'/>",
                        x - slot * 0.35 + bw * s as f32,
                        top + h - y
                    ));
                }
            }
            kind => {
                if kind != ChartKind::Scatter
                    && let (Some(first), Some(last)) = (pts.first(), pts.last())
                {
                    out.push_str(&format!(
                        "<polygon points='{},{} {} {},{}' fill='{color}' fill-opacity='{}'/>",
                        first.0,
                        top + h,
                        poly.join(" "),
                        last.0,
                        top + h,
                        if kind == ChartKind::Line { 0.08 } else { 0.14 }
                    ));
                }
                if kind != ChartKind::Scatter {
                    out.push_str(&format!(
                        "<polyline points='{}' fill='none' stroke='{color}' stroke-width='2'/>",
                        poly.join(" ")
                    ));
                }
                if model.show_points || kind == ChartKind::Scatter {
                    for (x, y) in &pts {
                        out.push_str(&format!("<circle cx='{x}' cy='{y}' r='3.5' fill='#ffffff' stroke='{color}' stroke-width='1.5'/>"));
                    }
                }
            }
        }
    }
    out.push_str("</svg>");
    out
}

/// Renders [`svg`] to PNG bytes with the bundled UI font.
pub(crate) fn png(model: &ChartModel, title: &str) -> Result<Vec<u8>, String> {
    let (width, height) = (1200.0, 600.0);
    let svg = svg(model, title, width, height);
    let mut db = resvg::usvg::fontdb::Database::new();
    for font in crate::assets::FONTS {
        db.load_font_data(font.to_vec());
    }
    let options = resvg::usvg::Options {
        fontdb: std::sync::Arc::new(db),
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width as u32, height as u32).ok_or("empty chart")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use types::{ChartData, ChartKind, ChartPoint, Series, Value};

    use super::{ChartModel, nice, png, short_number, x_labels};

    #[test]
    fn nice_ticks_and_labels() {
        assert_eq!(nice(54_000.0), (60_000.0, 20_000.0));
        assert_eq!(short_number(20_000.0), "20k");
        assert_eq!(short_number(1_200_000.0), "1.2M");
        let xs = ["2026-11-07", "2026-11-09", "2026-12-01"].map(|d| Value::Text(d.into()));
        assert_eq!(x_labels(&xs), ["Nov 7", "9", "Dec 1"]);
    }

    #[test]
    fn exports_png() {
        let model = ChartModel {
            kind: ChartKind::Area,
            show_points: true,
            data: ChartData {
                series: vec![Series {
                    name: "events".into(),
                    points: (1..5)
                        .map(|i| ChartPoint {
                            x: Value::Int(i),
                            y: i as f64 * 10.0,
                        })
                        .collect(),
                }],
                truncated: false,
            },
        };
        let bytes = png(&model, "Events per day").expect("png");
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
