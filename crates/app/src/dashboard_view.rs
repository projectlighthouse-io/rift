//! A dashboard document tab: pinned chart cards that refresh on their own timers and raise a
//! desktop notification when an alert rule starts firing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Context, Entity, FontWeight, SharedString, Subscription, Task, Window, anchored, deferred, div,
    px,
};
use gpui_component::input::{InputEvent, InputState};
use tokio_util::sync::CancellationToken;
use traits::Connection;
use types::{AlertOp, AlertRule, ChartData, Dashboard, Error};

use crate::chart::{self, ChartModel};
use crate::dropdown::{Changed, Dropdown};
use crate::theme::{self, c};
use crate::{runtime, state, ui};

const RANGES: [(&str, Option<i64>); 5] = [
    ("Last 24 hours", Some(1)),
    ("Last 7 days", Some(7)),
    ("Last 14 days", Some(14)),
    ("Last 30 days", Some(30)),
    ("All time", None),
];
const INTERVALS: [(&str, Option<u32>); 5] = [
    ("Manual", None),
    ("Every 1 min", Some(60)),
    ("Every 5 min", Some(300)),
    ("Every 15 min", Some(900)),
    ("Every 1 h", Some(3600)),
];
const CARD_HEIGHT: f32 = 260.;

#[derive(Default)]
struct CardState {
    data: Option<ChartData>,
    error: Option<String>,
    refreshed: Option<Instant>,
    running: bool,
    firing: bool,
    _task: Option<Task<()>>,
}

pub(crate) struct DashboardView {
    conn: Arc<dyn Connection>,
    pub(crate) dashboard: Dashboard,
    connection_name: SharedString,
    cards: Vec<CardState>,
    range: Entity<Dropdown>,
    menu: Option<usize>,
    alert_input: Option<(usize, Entity<InputState>)>,
    _ticker: Task<()>,
    _subscriptions: Vec<Subscription>,
}

fn interval_label(secs: Option<u32>) -> String {
    match secs {
        None => "manual".into(),
        Some(s) if s % 3600 == 0 => format!("every {} h", s / 3600),
        Some(s) => format!("every {} min", s / 60),
    }
}

/// Fires when the latest value crosses the rule.
fn crossed(rule: &AlertRule, value: f64) -> bool {
    match rule.op {
        AlertOp::Above => value > rule.threshold,
        // `AlertOp` is non-exhaustive; `Below` and anything newer compare as below.
        _ => value < rule.threshold,
    }
}

/// Best-effort desktop notification.
// ponytail: shells out to the OS notifier; a notification crate if we need actions or icons.
fn notify(title: &str, body: &str) {
    let (title, body) = (title.replace('"', "'"), body.replace('"', "'"));
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "display notification \"{body}\" with title \"Rift · {title}\""
        ))
        .spawn();
    #[cfg(not(target_os = "macos"))]
    let result = std::process::Command::new("notify-send")
        .arg(format!("Rift · {title}"))
        .arg(body)
        .spawn();
    if let Err(e) = result {
        tracing::warn!("notification: {e}");
    }
}

impl DashboardView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        dashboard: Dashboard,
        connection_name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let range =
            cx.new(|_| Dropdown::new(RANGES.iter().map(|r| r.0.into()).collect(), 2).height(30.));
        let range_changed = cx.subscribe_in(&range, window, |_, _, _: &Changed, _, cx| cx.notify());
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |view, cx| view.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let cards = dashboard
            .cards
            .iter()
            .map(|_| CardState::default())
            .collect();
        let mut view = Self {
            conn,
            dashboard,
            connection_name,
            cards,
            range,
            menu: None,
            alert_input: None,
            _ticker: ticker,
            _subscriptions: vec![range_changed],
        };
        view.refresh_all(cx);
        view
    }

    /// Picks up cards pinned since this tab opened.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let Ok(list) = state::dashboards(cx).list(self.dashboard.connection) else {
            return;
        };
        if let Some(d) = list.into_iter().find(|d| d.id == self.dashboard.id) {
            self.cards.resize_with(d.cards.len(), CardState::default);
            self.dashboard = d;
            self.refresh_all(cx);
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let due: Vec<usize> = (0..self.cards.len())
            .filter(|i| {
                let (card, s) = (&self.dashboard.cards[*i], &self.cards[*i]);
                match (card.refresh_secs, s.refreshed) {
                    (Some(secs), Some(at)) => {
                        !s.running && at.elapsed() >= Duration::from_secs(u64::from(secs))
                    }
                    _ => false,
                }
            })
            .collect();
        for i in due {
            tracing::info!(card = %self.dashboard.cards[i].title, "timed refresh");
            self.refresh(i, cx);
        }
    }

    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        for i in 0..self.cards.len() {
            self.refresh(i, cx);
        }
    }

    fn refresh(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(card) = self.dashboard.cards.get(i) else {
            return;
        };
        let request = card.chart.request.clone();
        let conn = self.conn.clone();
        self.cards[i].running = true;
        self.cards[i]._task = Some(runtime::spawn_db(
            cx,
            async move {
                match conn.aggregate() {
                    Some(a) => a.aggregate(&request, CancellationToken::new()).await,
                    None => Err(Error::unsupported("charts")),
                }
            },
            move |this: &mut Self, result, cx| {
                let Some(state) = this.cards.get_mut(i) else {
                    return;
                };
                state.running = false;
                state.refreshed = Some(Instant::now());
                match result {
                    Ok(data) => {
                        state.error = None;
                        let card = &this.dashboard.cards[i];
                        if let (Some(rule), Some((value, _))) = (&card.alert, chart::latest(&data))
                        {
                            let firing = crossed(rule, value);
                            if firing && !state.firing {
                                notify(&card.title, &format!("{} (now {value})", rule.label));
                            }
                            state.firing = firing;
                        }
                        state.data = Some(data);
                    }
                    Err(e) => state.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = state::dashboards(cx).save(&self.dashboard) {
            tracing::warn!("saving dashboard: {e}");
        }
        cx.notify();
    }

    fn set_interval(&mut self, i: usize, secs: Option<u32>, cx: &mut Context<Self>) {
        self.dashboard.cards[i].refresh_secs = secs;
        self.menu = None;
        self.save(cx);
    }

    fn remove(&mut self, i: usize, cx: &mut Context<Self>) {
        self.dashboard.cards.remove(i);
        self.cards.remove(i);
        self.menu = None;
        self.save(cx);
    }

    fn start_alert(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Alert if below…"));
        input.update(cx, |s, cx| s.focus(window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    if let Ok(threshold) = input.read(cx).value().trim().parse::<f64>() {
                        this.dashboard.cards[i].alert = Some(AlertRule {
                            op: AlertOp::Below,
                            threshold,
                            label: format!("Alert if < {threshold}"),
                        });
                        this.cards[i].firing = false;
                        this.save(cx);
                        this.refresh(i, cx);
                    }
                    this.alert_input = None;
                    cx.notify();
                }
            },
        ));
        self.alert_input = Some((i, input));
        cx.notify();
    }

    fn render_menu(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |id: SharedString, label: String| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(13.))
                .cursor_pointer()
                .hover(|d| d.bg(c(theme::SELECTION)))
                .child(label)
        };
        let has_alert = self.dashboard.cards[i].alert.is_some();
        deferred(
            anchored().snap_to_window().child(
                div()
                    .id("card-menu")
                    .occlude()
                    .mt(px(24.))
                    .w(px(170.))
                    .p(px(4.))
                    .rounded(px(6.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }))
                    .child(
                        item("menu-refresh".into(), "Refresh now".into()).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.menu = None;
                                this.refresh(i, cx);
                            },
                        )),
                    )
                    .children(INTERVALS.iter().map(|(label, secs)| {
                        let secs = *secs;
                        item(format!("menu-{label}").into(), (*label).to_owned()).on_click(
                            cx.listener(move |this, _, _, cx| this.set_interval(i, secs, cx)),
                        )
                    }))
                    .child(div().h(px(1.)).my(px(4.)).bg(c(theme::LINE)))
                    .child(if has_alert {
                        item("menu-alert".into(), "Remove alert".into()).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.dashboard.cards[i].alert = None;
                                this.cards[i].firing = false;
                                this.menu = None;
                                this.save(cx);
                            },
                        ))
                    } else {
                        item("menu-alert".into(), "Add alert…".into()).on_click(
                            cx.listener(move |this, _, window, cx| this.start_alert(i, window, cx)),
                        )
                    })
                    .child(
                        item("menu-remove".into(), "Remove card".into())
                            .text_color(c(theme::DELETED_TEXT))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove(i, cx))),
                    ),
            ),
        )
        .with_priority(1)
    }

    fn render_card(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let card = &self.dashboard.cards[i];
        let state = &self.cards[i];
        let mono = theme::mono(cx);
        let days = RANGES[self.range.read(cx).selected()].1;
        let body = match (&state.error, &state.data) {
            (Some(e), _) => div()
                .text_size(px(12.5))
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(e.clone()))
                .into_any_element(),
            (None, Some(data)) => chart::chart_view(
                ChartModel {
                    kind: card.chart.kind,
                    data: chart::within_days(data.clone(), days),
                    show_points: card.chart.show_points,
                },
                mono.clone(),
            ),
            (None, None) => div()
                .text_size(px(12.5))
                .text_color(c(theme::FAINT))
                .child("Loading…")
                .into_any_element(),
        };
        let alert = card.alert.as_ref().map(|rule| {
            let (fg, bg) = if state.firing {
                (theme::DELETED_TEXT, theme::DELETED_BG)
            } else {
                (theme::EDITED_TEXT, theme::EDITED_BG)
            };
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .h(px(20.))
                .rounded(px(4.))
                .bg(c(bg))
                .text_size(px(11.5))
                .text_color(c(fg))
                .child(ui::icon("bell", 11., fg))
                .child(SharedString::from(rule.label.clone()))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .h(px(CARD_HEIGHT))
            .p(px(16.))
            .rounded(px(8.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(card.title.clone())),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child(SharedString::from(card.subtitle.clone())),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .children(alert)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .text_size(px(12.))
                                    .text_color(c(theme::MUTED))
                                    .child(ui::icon("refresh", 11., theme::MUTED))
                                    .child(SharedString::from(interval_label(card.refresh_secs))),
                            )
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        ui::icon_button(("card-more", i), "more", 22., 14.)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.menu = if this.menu == Some(i) {
                                                    None
                                                } else {
                                                    Some(i)
                                                };
                                                cx.notify();
                                            })),
                                    )
                                    .children(
                                        (self.menu == Some(i)).then(|| self.render_menu(i, cx)),
                                    ),
                            ),
                    ),
            )
            .children(
                self.alert_input
                    .as_ref()
                    .filter(|(ix, _)| *ix == i)
                    .map(|(_, input)| {
                        div()
                            .h(px(30.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .child(ui::input(input, 13.))
                    }),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl Render for DashboardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.dashboard.cards.len();
        let header =
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .h(px(48.))
                .px(px(16.))
                .bg(c(theme::BG))
                .border_b_1()
                .border_color(c(theme::LINE))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(self.dashboard.title.clone())),
                )
                .child(div().text_size(px(12.)).text_color(c(theme::MUTED)).child(
                    SharedString::from(format!(
                        "{count} chart{} · {}",
                        if count == 1 { "" } else { "s" },
                        self.connection_name
                    )),
                ))
                .child(
                    div()
                        .ml_auto()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(130.)).child(self.range.clone()))
                        .child(
                            ui::secondary_button("refresh-all", "Refresh all")
                                .flex_row_reverse()
                                .child(ui::icon("refresh", 13., theme::TEXT))
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx))),
                        )
                        .child(
                            // ponytail: charts are built in a query's Chart tab and pinned here.
                            ui::primary_button("add-chart", "Add chart")
                                .flex_row_reverse()
                                .child(ui::icon("plus", 13., theme::BG))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(AddChart))),
                        ),
                );
        let cards: Vec<_> = (0..count)
            .map(|i| self.render_card(i, cx).into_any_element())
            .collect();
        div().flex().flex_col().size_full().child(header).child(
            div()
                .id("dashboard-cards")
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(ui::dot_grid())
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap(px(16.))
                        .p(px(16.))
                        .children(cards)
                        .when(count == 0, |d| {
                            d.child(div().text_size(px(13.)).text_color(c(theme::MUTED)).child(
                                "No charts yet. Run a query, open its Chart tab and pin it.",
                            ))
                        }),
                ),
        )
    }
}

/// Asks the session for a new query tab to build a chart in.
pub(crate) struct AddChart;

impl gpui::EventEmitter<AddChart> for DashboardView {}

#[cfg(test)]
mod tests {
    use types::{AlertOp, AlertRule};

    #[test]
    fn alert_crossing_and_interval_labels() {
        let rule = AlertRule {
            op: AlertOp::Below,
            threshold: 30.0,
            label: String::new(),
        };
        assert!(super::crossed(&rule, 29.0));
        assert!(!super::crossed(&rule, 30.0));
        assert_eq!(super::interval_label(Some(300)), "every 5 min");
        assert_eq!(super::interval_label(Some(3600)), "every 1 h");
    }
}
