//! Usage: what agents used, in tokens, and what it likely cost, over a range of days. Tiles for the
//! totals, a bar a day, and breakdowns by model, agent and source; pressing a line narrows the whole
//! screen to it. A model with no price can be given one here.

use rune::{El, ScrollSpec, TextStyle, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use super::providers::section;
use crate::app::App;
use crate::models::ModelRef;
use crate::usage::{Cost, Day, Group, Metric, Range, Report, Source, Totals, report, today};

const PAGE: f32 = 960.0;
const CHART: f32 = 140.0;

pub fn screen(s: &App, k: &Kit) -> El {
    let u = &s.usage;
    let r = report(&u.rows, &s.prices, &u.filters, u.range, today());
    let mut page = col()
        .w_full()
        .max_w(PAGE)
        .px(24.0)
        .py(32.0)
        .gap(24.0)
        .child(heading(k))
        .child(controls(s, k));
    if !u.filters.is_empty() {
        page = page.child(chips(s, &r, k));
    }
    if r.totals.calls == 0 {
        page = page.child(empty(s, k));
    } else {
        page = page
            .child(tiles(&r, k))
            .child(chart(&r, u.metric, k))
            .child(models(s, &r, k))
            .child(agents(s, &r, k))
            .child(sources(s, &r, k));
    }
    col()
        .key("usage")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn heading(k: &Kit) -> El {
    col()
        .gap(10.0)
        .child(
            text("Usage")
                .font(TextStyle {
                    size: 28.0,
                    line_height: 34.0,
                    ..k.ty.body_medium
                })
                .color(k.c.text_base),
        )
        .child(
            text("What your agents used, in tokens, and what it likely cost. Costs are estimates from models.dev's prices, or yours where you set one; your provider's bill is what counts.")
                .font(k.ty.body)
                .color(k.c.text_muted)
                .wrap()
                .max_w(620.0),
        )
}

fn controls(s: &App, k: &Kit) -> El {
    let u = &s.usage;
    let labels: Vec<&str> = Range::ALL.iter().map(|r| r.label()).collect();
    let range = SegmentedControl::new(&labels, Range::ALL.iter().position(|r| *r == u.range))
        .label("Range")
        .width(Some(360.0))
        .on_pick(|s: &mut App, i, _| s.set_usage_range(Range::ALL[i.min(Range::ALL.len() - 1)]))
        .build(k)
        .key("usage-range");
    let metric = SegmentedControl::new(
        &["Cost", "Tokens"],
        Some(usize::from(u.metric == Metric::Tokens)),
    )
    .label("Chart")
    .on_pick(|s: &mut App, i, _| {
        s.set_usage_metric(if i == 0 { Metric::Cost } else { Metric::Tokens });
    })
    .build(k)
    .key("usage-metric");
    row()
        .w_full()
        .center_y()
        .gap(12.0)
        .child(range)
        .child(el().grow(1.0))
        .child(metric)
}

/// The filters in force, each one pressed to drop it.
fn chips(s: &App, r: &Report, k: &Kit) -> El {
    let f = &s.usage.filters;
    let mut out = row().key("usage-filters").w_full().center_y().gap(8.0);
    if let Some(m) = &f.model {
        let m = m.clone();
        out = out.child(chip(
            format!("Model: {}", model_name(s, &m)),
            "usage-filter-model",
            k,
            move |s| s.filter_usage_model(&m),
        ));
    }
    if let Some(a) = &f.agent {
        let name = r.agent_names.get(a).cloned().unwrap_or_else(|| a.clone());
        let a = a.clone();
        out = out.child(chip(
            format!("Agent: {name}"),
            "usage-filter-agent",
            k,
            move |s| s.filter_usage_agent(&a),
        ));
    }
    if let Some(src) = f.source {
        out = out.child(chip(
            format!("Source: {}", source_name(src)),
            "usage-filter-source",
            k,
            move |s| s.filter_usage_source(src),
        ));
    }
    out.child(
        Button::new("Clear filters")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.clear_usage_filters())
            .build(k)
            .key("usage-filter-clear"),
    )
}

fn chip(label: String, key: &str, k: &Kit, on: impl Fn(&mut App) + 'static) -> El {
    let e = row()
        .key(key.to_owned())
        .px(12.0)
        .py(6.0)
        .gap(6.0)
        .radius(16.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
        .role("button")
        .label(format!("Remove filter {label}"))
        .on_press(on)
        .child(text(label).font(k.ty.small).color(k.c.text_base))
        .child(text("×").font(k.ty.small).color(k.c.text_muted));
    k.interactive(e)
}

fn empty(s: &App, k: &Kit) -> El {
    let why = if s.usage.filters.is_empty() {
        "Nothing used in this range yet. Every model call your agents make is counted here, and kept until you clear it in Settings."
    } else {
        "Nothing matches these filters in this range."
    };
    col().key("usage-empty").w_full().py(48.0).center().child(
        text(why)
            .font(k.ty.small)
            .color(k.c.text_muted)
            .wrap()
            .max_w(420.0),
    )
}

fn tiles(r: &Report, k: &Kit) -> El {
    let t = &r.totals;
    let pct = |v: Option<f64>| v.map_or_else(|| "—".to_owned(), |v| format!("{:.0}%", v * 100.0));
    let per_day = t.calls as f64 / r.days.len().max(1) as f64;
    let calls = format!("about {per_day:.0} a day");
    let tiles = [
        ("Cost", money(&r.cost), cost_note(&r.cost)),
        (
            "Tokens",
            tokens(t.tokens()),
            format!("{} in · {} out", tokens(t.input), tokens(t.output)),
        ),
        ("Calls", t.calls.to_string(), calls),
        (
            "Cache hits",
            pct(t.cache_hit_rate()),
            format!("{} of input read from cache", tokens(t.cache_read)),
        ),
        (
            "First token",
            t.avg_first_ms().map_or_else(|| "—".to_owned(), millis),
            "average wait for a reply to start".to_owned(),
        ),
        (
            "Errors",
            pct(t.error_rate()),
            match t.errors {
                1 => "1 failed call".to_owned(),
                n => format!("{n} failed calls"),
            },
        ),
    ];
    let tile = |(label, value, note): &(&str, String, String)| {
        col()
            .key(format!(
                "usage-tile-{}",
                label.to_lowercase().replace(' ', "-")
            ))
            .grow(1.0)
            .min_w(0.0)
            .padding(16.0)
            .gap(4.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_muted)
            .role("group")
            .label(format!("{label}: {value}"))
            .child(
                text(label.to_string())
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            )
            .child(
                text(value.clone())
                    .font(TextStyle {
                        size: 22.0,
                        line_height: 28.0,
                        ..k.ty.body_medium
                    })
                    .color(k.c.text_base)
                    .truncate(),
            )
            .child(
                text(note.clone())
                    .font(k.ty.small)
                    .color(k.c.text_faint)
                    .truncate(),
            )
    };
    let line =
        |part: &[(&str, String, String)]| row().w_full().gap(12.0).children(part.iter().map(tile));
    col()
        .w_full()
        .gap(12.0)
        .child(line(&tiles[..3]))
        .child(line(&tiles[3..]))
}

/// A bar a day, scaled to the biggest; each says its day and numbers when pointed at.
fn chart(r: &Report, metric: Metric, k: &Kit) -> El {
    let value = |d: &Day| match metric {
        Metric::Cost => d.cost.dollars,
        Metric::Tokens => d.totals.tokens() as f64,
    };
    let top = r.days.iter().map(value).fold(0.0, f64::max);
    let bars = r.days.iter().map(|d| {
        let v = value(d);
        let h = if top > 0.0 {
            (v / top * f64::from(CHART)) as f32
        } else {
            0.0
        };
        let mut bar = el().w_full().h(h.max(if v > 0.0 { 2.0 } else { 0.0 }));
        if v > 0.0 {
            bar = bar.radius(2.0).fill(k.c.bg_accent);
        }
        let help = format!(
            "{} · {} · {} tokens · {} calls",
            d.day,
            money(&d.cost),
            tokens(d.totals.tokens()),
            d.totals.calls
        );
        col()
            .key(format!("usage-day-{}", d.day))
            .grow(1.0)
            .min_w(0.0)
            .h(CHART)
            .role("img")
            .label(help.clone())
            .help(help)
            .child(el().grow(1.0))
            .child(bar)
    });
    let first = r.days.first().map(|d| d.day.clone()).unwrap_or_default();
    let last = r.days.last().map(|d| d.day.clone()).unwrap_or_default();
    let peak = match metric {
        _ if top <= 0.0 => String::new(),
        Metric::Cost => format!(
            "Peak {}",
            money(&Cost {
                dollars: top,
                partial: false
            })
        ),
        Metric::Tokens => format!("Peak {} tokens", tokens(top as u64)),
    };
    col()
        .key("usage-chart")
        .w_full()
        .padding(16.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            row()
                .w_full()
                .child(
                    text(match metric {
                        Metric::Cost => "Cost a day",
                        Metric::Tokens => "Tokens a day",
                    })
                    .font(k.ty.small_medium)
                    .color(k.c.text_muted),
                )
                .child(el().grow(1.0))
                .child(text(peak).font(k.ty.small).color(k.c.text_faint)),
        )
        .child(if top > 0.0 {
            row().w_full().h(CHART).gap(2.0).children(bars)
        } else {
            row().w_full().h(CHART).center().child(
                text("None of these calls has a price. Set one below, or chart tokens instead.")
                    .key("usage-chart-none")
                    .font(k.ty.small)
                    .color(k.c.text_faint),
            )
        })
        .child(
            row()
                .w_full()
                .child(text(first).font(k.ty.small).color(k.c.text_faint))
                .child(el().grow(1.0))
                .child(text(last).font(k.ty.small).color(k.c.text_faint)),
        )
}

fn models(s: &App, r: &Report, k: &Kit) -> El {
    let rows = r
        .models
        .iter()
        .flat_map(|g| {
            let m = g.key.clone();
            let open = s.usage.price.as_ref().is_some_and(|f| f.model == g.key);
            let selected = s.usage.filters.model.as_ref() == Some(&g.key);
            let line = line(
                format!("usage-model-{}-{}", g.key.provider, g.key.id),
                model_name(s, &g.key),
                format!(
                    "{} · {}",
                    s.providers.label_of(&g.key.provider),
                    detail(&g.totals)
                ),
                g,
                selected,
                Some(price_button(s, &g.key, open, k)),
                k,
                move |s| s.filter_usage_model(&m),
            );
            std::iter::once(line).chain(open.then(|| price_form(s, k)))
        })
        .collect();
    section("By model", r.models.len(), rows, k)
}

fn agents(s: &App, r: &Report, k: &Kit) -> El {
    let rows = r
        .agents
        .iter()
        .map(|g| {
            let id = g.key.clone();
            let name = r
                .agent_names
                .get(&g.key)
                .cloned()
                .unwrap_or_else(|| g.key.clone());
            line(
                format!("usage-agent-{}", g.key),
                name,
                detail(&g.totals),
                g,
                s.usage.filters.agent.as_ref() == Some(&g.key),
                None,
                k,
                move |s| s.filter_usage_agent(&id),
            )
        })
        .collect();
    section("By agent", r.agents.len(), rows, k)
}

fn sources(s: &App, r: &Report, k: &Kit) -> El {
    let rows = r
        .sources
        .iter()
        .map(|g| {
            let src = g.key;
            line(
                format!("usage-source-{}", src.as_str()),
                source_name(src).to_owned(),
                detail(&g.totals),
                g,
                s.usage.filters.source == Some(src),
                None,
                k,
                move |s| s.filter_usage_source(src),
            )
        })
        .collect();
    section("By source", r.sources.len(), rows, k)
}

/// One breakdown line: its name and numbers, pressed to narrow the screen to it; then its cost and
/// any action.
#[allow(clippy::too_many_arguments)]
fn line<K>(
    key: String,
    name: String,
    detail: String,
    g: &Group<K>,
    selected: bool,
    action: Option<El>,
    k: &Kit,
    on: impl Fn(&mut App) + 'static,
) -> El {
    let c = &k.c;
    let label = format!(
        "{name}: {}. {}",
        money(&g.cost),
        if selected {
            "Showing only this; press to show all"
        } else {
            "Press to show only this"
        }
    );
    let named = k.interactive(
        col()
            .key(format!("{key}-name"))
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .role("button")
            .label(label)
            .on_press(on)
            .child(
                text(if selected {
                    format!("● {name}")
                } else {
                    name
                })
                .font(k.ty.ui_medium)
                .color(if selected { c.text_accent } else { c.text_base })
                .truncate(),
            )
            .child(text(detail).font(k.ty.small).color(c.text_faint).truncate()),
    );
    row()
        .key(key)
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(16.0)
        .center_y()
        .child(named)
        .child(
            text(money(&g.cost))
                .font(k.ty.ui_medium)
                .color(if g.cost.unknown() {
                    c.text_faint
                } else {
                    c.text_base
                }),
        )
        .children(action)
}

/// "Set price" for a model with none, "Your price" for one you set, "Edit price" otherwise.
fn price_button(s: &App, m: &ModelRef, open: bool, k: &Kit) -> El {
    let label = if open {
        "Close"
    } else if s.prices.manual.contains_key(m) {
        "Your price"
    } else if s.prices.get(m).is_some() {
        "Edit price"
    } else {
        "Set price"
    };
    let on = m.clone();
    Button::new(label)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .help("US dollars per million tokens, used for this model in place of the catalog's")
        .on_press(move |s: &mut App| s.toggle_price_form(&on))
        .build(k)
        .key(format!("usage-price-{}-{}", m.provider, m.id))
}

fn price_form(s: &App, k: &Kit) -> El {
    let Some(f) = &s.usage.price else {
        return el();
    };
    let submit = |s: &mut App, _: &mut rune::Tx| s.save_price();
    let field = |label: &str,
                 key: &str,
                 edit: &rune::TextEdit,
                 placeholder: &str,
                 pick: fn(&mut App) -> &mut rune::TextEdit| {
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(4.0)
            .child(
                text(label.to_owned())
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            )
            .child(
                TextInput::new(edit)
                    .label(label)
                    .placeholder(placeholder)
                    .width(None)
                    .on_edit(pick)
                    .on_submit(submit)
                    .build(k)
                    .key(format!("usage-price-{key}")),
            )
    };
    let fields = row()
        .w_full()
        .gap(8.0)
        .child(field("Input", "input", &f.input, "e.g. 3", |s| {
            &mut s.usage.price_mut().input
        }))
        .child(field("Output", "output", &f.output, "e.g. 15", |s| {
            &mut s.usage.price_mut().output
        }))
        .child(field(
            "Cache read",
            "cache-read",
            &f.cache_read,
            "as input",
            |s| &mut s.usage.price_mut().cache_read,
        ))
        .child(field(
            "Cache write",
            "cache-write",
            &f.cache_write,
            "as input",
            |s| &mut s.usage.price_mut().cache_write,
        ));
    let mut buttons = row().w_full().gap(8.0).center_y().child(
        Button::new("Save price")
            .variant(ButtonVariant::Submit)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.save_price())
            .build(k)
            .key("usage-price-save"),
    );
    if s.prices.manual.contains_key(&f.model) {
        let m = f.model.clone();
        let back = if s.prices.catalog.contains_key(&m) {
            "Use the catalog's price"
        } else {
            "Remove my price"
        };
        buttons = buttons.child(
            Button::new(back)
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.reset_price(&m))
                .build(k)
                .key("usage-price-reset"),
        );
    }
    let mut form = col()
        .key("usage-price-form")
        .w_full()
        .px(16.0)
        .pt(4.0)
        .pb(16.0)
        .gap(10.0)
        .child(
            text("US dollars per million tokens. Leave a cache price blank to charge cached input as input.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
        )
        .child(fields);
    if let Some(why) = &f.error {
        form = form.child(
            text(why.clone())
                .key("usage-price-error")
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }
    form.child(buttons)
}

/// A model's name as its provider lists it, else its id.
fn model_name(s: &App, m: &ModelRef) -> String {
    s.providers
        .model_info(m)
        .map(|i| i.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| m.id.clone())
}

fn source_name(s: Source) -> &'static str {
    match s {
        Source::Chat => "Chats",
        Source::Schedule => "Schedules",
        Source::Workflow => "Workflows",
        Source::Member => "Team members",
    }
}

/// "1.2M tokens · 40 calls · 12% from cache".
fn detail(t: &Totals) -> String {
    let mut parts = vec![
        format!("{} tokens", tokens(t.tokens())),
        match t.calls {
            1 => "1 call".to_owned(),
            n => format!("{n} calls"),
        },
    ];
    if let Some(rate) = t.cache_hit_rate().filter(|r| *r > 0.0) {
        parts.push(format!("{:.0}% from cache", rate * 100.0));
    }
    parts.join(" · ")
}

/// "$12.34", "$0.0042", "<$0.0001", "$1.20+" when some had no price, "—" when none had.
pub fn money(c: &Cost) -> String {
    if c.unknown() {
        return "—".to_owned();
    }
    let d = c.dollars;
    let amount = match d {
        0.0 => "$0".to_owned(),
        d if d < 0.0001 => "<$0.0001".to_owned(),
        d if d < 0.01 => format!("${d:.4}"),
        d => format!("${d:.2}"),
    };
    if c.partial {
        format!("{amount}+")
    } else {
        amount
    }
}

fn cost_note(c: &Cost) -> String {
    if c.unknown() {
        "no prices for these models; set one below".to_owned()
    } else if c.partial {
        "some models have no price; set one below".to_owned()
    } else {
        "estimated from current prices".to_owned()
    }
}

/// "812", "12.4k", "3.2M".
pub fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn millis(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{:.1} s", ms as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_says_what_is_known_and_marks_what_is_not() {
        let c = |dollars, partial| Cost { dollars, partial };
        assert_eq!(money(&c(12.345, false)), "$12.35");
        assert_eq!(money(&c(0.18, false)), "$0.18");
        assert_eq!(money(&c(0.00421, false)), "$0.0042");
        assert_eq!(money(&c(0.00001, false)), "<$0.0001");
        assert_eq!(money(&c(0.0, false)), "$0");
        assert_eq!(money(&c(1.2, true)), "$1.20+");
        assert_eq!(money(&c(0.0, true)), "—");
    }

    #[test]
    fn token_counts_shorten() {
        assert_eq!(tokens(812), "812");
        assert_eq!(tokens(12_400), "12.4k");
        assert_eq!(tokens(3_200_000), "3.2M");
    }
}
