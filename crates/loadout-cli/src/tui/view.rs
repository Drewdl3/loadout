//! Drawing the TUI.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap,
};
use serde_json::Value;

use super::app::{App, FACETS, Prompt, Tab};

const ACCENT: Color = Color::Cyan;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
}

fn state_span(state: &str) -> Span<'static> {
    let (label, color) = match state {
        "enabled" => ("● on  ", Color::Green),
        "disabled" => ("○ off ", Color::DarkGray),
        "shadowed" => ("◐ shad", Color::Yellow),
        "not-subscribed" => ("+ join", ACCENT),
        _ => ("· n/a ", Color::Blue),
    };
    Span::styled(label, Style::new().fg(color))
}

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(f.area());

    let selected = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    let pending = app.status["pending"].as_array().map_or(0, Vec::len);
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut l = vec![Span::raw(format!("{} {}", i + 1, t.title()))];
            if *t == Tab::Status && pending > 0 {
                l.push(Span::styled(
                    format!(" ({pending})"),
                    Style::new().fg(Color::Yellow),
                ));
            }
            Line::from(l)
        })
        .collect();
    let [brand, tabs_area] =
        Layout::horizontal([Constraint::Length(11), Constraint::Min(10)]).areas(header);
    f.render_widget(
        Paragraph::new(Span::styled(" ◆ Loadout", Style::new().fg(ACCENT).bold())),
        brand,
    );
    f.render_widget(
        Tabs::new(titles)
            .select(selected)
            .highlight_style(Style::new().fg(ACCENT).bold().underlined())
            .divider(Span::styled("│", Style::new().fg(Color::DarkGray))),
        tabs_area,
    );

    match app.tab {
        Tab::Catalog => catalog(f, app, body),
        Tab::Sources => sources(f, app, body),
        Tab::Templates => templates(f, app, body),
        Tab::Groups => groups(f, app, body),
        Tab::Status => status(f, app, body),
    }

    let hints = match app.tab {
        Tab::Catalog => "/ search  s k o r t filter  c clear  a all groups  space toggle  w why",
        Tab::Sources => "n preview+add  u unsubscribe  enter items",
        Tab::Templates => "enter use  x open pull request",
        Tab::Groups => "enter join/leave",
        Tab::Status => "a approve  A approve all",
    };
    let msg = match &app.message {
        Some((m, true)) => Line::from(Span::styled(format!(" {m}"), Style::new().fg(Color::Red))),
        Some((m, false)) => {
            Line::from(Span::styled(format!(" {m}"), Style::new().fg(Color::Green)))
        }
        None => Line::from(Span::styled(
            format!(
                " {}  ·  last sync {}",
                s(&app.status["fingerprint"]),
                app.status["last_sync"].as_str().unwrap_or("never")
            ),
            Style::new().fg(Color::DarkGray),
        )),
    };
    f.render_widget(
        Paragraph::new(vec![
            msg,
            Line::from(vec![
                Span::styled(format!(" {hints}"), Style::new().fg(Color::Gray)),
                Span::styled(
                    "   S sync  R refresh  ? help  q quit",
                    Style::new().fg(Color::DarkGray),
                ),
            ]),
        ]),
        footer,
    );

    if let Some(input) = &app.input {
        let title = match input.prompt {
            Prompt::Search => "Search",
            Prompt::SourceUrl => "Preview a source (Git URL or path)",
        };
        let area = popup(f.area(), 70, 3);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(format!("{}▏", input.text)).block(block(title)),
            area,
        );
    }
    if app.form.is_some() {
        form(f, app);
    }
    if app.help {
        help(f);
    }
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(4));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 3,
        width: w,
        height: h,
    }
}

fn catalog(f: &mut Frame, app: &App, area: Rect) {
    let [filters, main] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(area);
    let mut spans = vec![Span::styled(" search ", Style::new().fg(Color::DarkGray))];
    spans.push(if app.query.is_empty() {
        Span::styled("—", Style::new().fg(Color::DarkGray))
    } else {
        Span::styled(format!("\"{}\"", app.query), Style::new().fg(ACCENT))
    });
    for (facet, sel) in FACETS.iter().zip(&app.filters) {
        spans.push(Span::styled(
            format!("  {}:", facet.name),
            Style::new().fg(Color::DarkGray),
        ));
        spans.push(match sel {
            Some(v) => Span::styled(v.clone(), Style::new().fg(ACCENT).bold()),
            None => Span::styled("any", Style::new().fg(Color::DarkGray)),
        });
    }
    if app.all_sources {
        spans.push(Span::styled(
            "  +groups you're not in",
            Style::new().fg(ACCENT),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), filters);

    let [list, detail] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(main);
    let visible = app.visible();
    let rows: Vec<Row> = visible
        .iter()
        .map(|d| {
            Row::new(vec![
                Cell::from(state_span(s(&d["state"]))),
                Cell::from(Span::styled(
                    s(&d["kind"]).to_owned(),
                    Style::new().fg(Color::DarkGray),
                )),
                Cell::from(Span::styled(s(&d["name"]).to_owned(), Style::new().bold())),
                Cell::from(Span::styled(
                    format!(
                        "{} · {}:{}",
                        s(&d["source"]),
                        s(&d["layer"]),
                        d["group"].as_str().unwrap_or("-")
                    ),
                    Style::new().fg(Color::Gray),
                )),
            ])
        })
        .collect();
    let title = format!("Items {}/{}", visible.len(), app.items.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Percentage(40),
            Constraint::Percentage(60),
        ],
    )
    .block(block(&title))
    .row_highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut st = TableState::default().with_selected((!visible.is_empty()).then_some(app.item_sel));
    f.render_stateful_widget(table, list, &mut st);

    let mut text = Text::default();
    if let Some(it) = app.selected_item() {
        text.push_line(Line::from(vec![
            Span::styled(s(&it["name"]).to_owned(), Style::new().bold().fg(ACCENT)),
            Span::raw("  "),
            state_span(s(&it["state"])),
        ]));
        text.push_line(Span::styled(
            s(&it["id"]).to_owned(),
            Style::new().fg(Color::DarkGray),
        ));
        text.push_line("");
        if let Some(d) = it["description"].as_str() {
            text.push_line(d.to_owned());
            text.push_line("");
        }
        let chips = |label: &str, v: &Value| -> Option<Line<'static>> {
            let vals: Vec<&str> = v.as_array()?.iter().filter_map(Value::as_str).collect();
            (!vals.is_empty()).then(|| {
                Line::from(vec![
                    Span::styled(format!("{label}: "), Style::new().fg(Color::DarkGray)),
                    Span::raw(vals.join(", ")),
                ])
            })
        };
        text.extend(chips("tags", &it["tags"]));
        text.extend(chips("roles", &it["roles"]));
        if let Some(w) = &app.why {
            text.push_line("");
            text.push_line(Span::styled("Why", Style::new().bold()));
            for c in w["candidates"].as_array().into_iter().flatten() {
                let winner = c["id"] == w["winner"];
                text.push_line(Line::from(vec![
                    Span::styled(
                        if winner { "✓ " } else { "  " },
                        Style::new().fg(Color::Green),
                    ),
                    Span::raw(s(&c["id"]).to_owned()),
                    Span::styled(
                        format!(
                            "  {}:{} · {}",
                            s(&c["layer"]),
                            c["group"].as_str().unwrap_or("-"),
                            s(&c["outcome"])
                        ),
                        Style::new().fg(Color::DarkGray),
                    ),
                ]));
            }
        } else {
            text.push_line("");
            text.push_line(Span::styled(
                "w: why this version",
                Style::new().fg(Color::DarkGray),
            ));
        }
    } else {
        text.push_line(Span::styled(
            "Nothing matches. c clears filters.",
            Style::new().fg(Color::DarkGray),
        ));
    }
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(block("Details")),
        detail,
    );
}

fn sources(f: &mut Frame, app: &App, area: Rect) {
    let [list, detail] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let rows: Vec<Row> = app
        .sources
        .iter()
        .map(|(depth, src)| {
            let prefix = if *depth == 0 {
                String::new()
            } else {
                format!("{}└ ", "  ".repeat(depth - 1))
            };
            let badge = if src["via"].is_string() {
                Span::styled("upstream", Style::new().fg(ACCENT))
            } else if src["manual"] == true {
                Span::styled("subscribed", Style::new().fg(Color::Green))
            } else {
                Span::styled("company_config", Style::new().fg(Color::Gray))
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::DarkGray)),
                    Span::styled(s(&src["name"]).to_owned(), Style::new().bold()),
                ])),
                Cell::from(badge),
                Cell::from(Span::styled(
                    s(&src["commit"]).chars().take(8).collect::<String>(),
                    Style::new().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Min(20),
            Constraint::Length(11),
            Constraint::Length(9),
        ],
    )
    .block(block("Sources"))
    .row_highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut st =
        TableState::default().with_selected((!app.sources.is_empty()).then_some(app.source_sel));
    f.render_stateful_widget(table, list, &mut st);

    let mut text = Text::default();
    if let Some((url, p)) = &app.preview {
        text.push_line(Span::styled(
            format!("Preview of {url}"),
            Style::new().bold().fg(ACCENT),
        ));
        for src in p["sources"].as_array().into_iter().flatten() {
            text.push_line("");
            let mut head = vec![
                Span::styled(s(&src["name"]).to_owned(), Style::new().bold()),
                Span::styled(
                    format!(
                        "  {}:{}",
                        s(&src["layer"]),
                        src["group"].as_str().unwrap_or("-")
                    ),
                    Style::new().fg(Color::Gray),
                ),
            ];
            if let Some(v) = src["via"].as_str() {
                head.push(Span::styled(
                    format!("  upstream of {v}"),
                    Style::new().fg(ACCENT),
                ));
            }
            text.push_line(Line::from(head));
            for it in src["items"].as_array().into_iter().flatten() {
                text.push_line(Line::from(vec![
                    Span::styled(
                        format!("  {:<6} ", s(&it["kind"])),
                        Style::new().fg(Color::DarkGray),
                    ),
                    Span::raw(s(&it["name"]).to_owned()),
                ]));
            }
            for t in src["templates"].as_array().into_iter().flatten() {
                text.push_line(Line::from(vec![
                    Span::styled("  tmpl   ", Style::new().fg(Color::DarkGray)),
                    Span::raw(s(t).rsplit('/').next().unwrap_or_default().to_owned()),
                ]));
            }
        }
        text.push_line("");
        text.push_line(Span::styled(
            "y subscribe and sync · esc discard",
            Style::new().fg(Color::Yellow),
        ));
    } else if let Some((_, src)) = app.sources.get(app.source_sel) {
        text.push_line(Span::styled(
            s(&src["name"]).to_owned(),
            Style::new().bold().fg(ACCENT),
        ));
        text.push_line(s(&src["url"]).to_owned());
        text.push_line(Span::styled(
            format!("commit {}", s(&src["commit"])),
            Style::new().fg(Color::DarkGray),
        ));
        if let Some(v) = src["via"].as_str() {
            text.push_line(format!("Pulled in by {v}'s upstream list."));
        }
        let ups: Vec<&str> = src["upstream"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !ups.is_empty() {
            text.push_line("");
            text.push_line(Span::styled("Builds on", Style::new().bold()));
            for u in ups {
                text.push_line(format!("  {u}"));
            }
        }
        if let Some(c) = &app.company_config {
            text.push_line("");
            text.push_line(Span::styled(
                format!("Company config: {c}"),
                Style::new().fg(Color::DarkGray),
            ));
        }
    } else {
        text.push_line("No sources yet. Press n to preview one and subscribe.");
    }
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(block(if app.preview.is_some() {
                "Preview"
            } else {
                "Details"
            })),
        detail,
    );
}

fn templates(f: &mut Frame, app: &App, area: Rect) {
    let [list, detail] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let rows: Vec<Row> = app
        .templates
        .iter()
        .map(|t| {
            Row::new(vec![
                Cell::from(Span::styled(s(&t["name"]).to_owned(), Style::new().bold())),
                Cell::from(Span::styled(
                    s(&t["source"]).to_owned(),
                    Style::new().fg(Color::Gray),
                )),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Percentage(55), Constraint::Percentage(45)],
    )
    .block(block("Templates"))
    .row_highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut st = TableState::default()
        .with_selected((!app.templates.is_empty()).then_some(app.template_sel));
    f.render_stateful_widget(table, list, &mut st);

    let mut text = Text::default();
    if let Some(t) = app.templates.get(app.template_sel) {
        text.push_line(Span::styled(
            s(&t["id"]).to_owned(),
            Style::new().bold().fg(ACCENT),
        ));
        if let Some(d) = t["description"].as_str() {
            text.push_line("");
            text.push_line(d.to_owned());
        }
        text.push_line("");
        text.push_line(Span::styled("Variables", Style::new().bold()));
        for v in t["variables"].as_array().into_iter().flatten() {
            text.push_line(Line::from(vec![
                Span::raw(format!("  {}", s(&v["name"]))),
                Span::styled(
                    match v["default"].as_str() {
                        Some(d) => format!("  default {d}"),
                        None => "  required".to_owned(),
                    },
                    Style::new().fg(Color::DarkGray),
                ),
                Span::styled(
                    v["description"]
                        .as_str()
                        .map(|d| format!("  {d}"))
                        .unwrap_or_default(),
                    Style::new().fg(Color::Gray),
                ),
            ]));
        }
        text.push_line("");
        text.push_line(Span::styled(
            "enter: fill it in and create your own skill in a source you can push to",
            Style::new().fg(Color::DarkGray),
        ));
    } else {
        text.push_line(
            "No templates in your sources. A source publishes them under templates/<name>/.",
        );
    }
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(block("Details")),
        detail,
    );
}

fn groups(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .groups
        .iter()
        .map(|u| {
            let member = u["member"] == true;
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{}:{}", s(&u["layer"]), s(&u["group"])),
                    Style::new().bold(),
                )),
                Cell::from(if member {
                    Span::styled("● member", Style::new().fg(Color::Green))
                } else {
                    Span::styled("○ not a member", Style::new().fg(Color::DarkGray))
                }),
                Cell::from(Span::styled(
                    s(&u["basis"]).to_owned(),
                    Style::new().fg(Color::Gray),
                )),
                Cell::from(Span::styled(
                    if member {
                        "leave"
                    } else if u["joinable"] == true {
                        "join"
                    } else {
                        ""
                    },
                    Style::new().fg(ACCENT),
                )),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Length(16),
            Constraint::Min(10),
            Constraint::Length(6),
        ],
    )
    .block(block("Your groups"))
    .row_highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut st =
        TableState::default().with_selected((!app.groups.is_empty()).then_some(app.group_sel));
    f.render_stateful_widget(table, area, &mut st);
}

fn status(f: &mut Frame, app: &App, area: Rect) {
    let [summary, pending] =
        Layout::vertical([Constraint::Length(6), Constraint::Min(3)]).areas(area);
    let st = &app.status;
    let conflicts = st["conflicts"] == true;
    let text = vec![
        Line::from(vec![
            Span::styled(
                format!("{}", st["enabled_items"].as_u64().unwrap_or(0)),
                Style::new().bold().fg(ACCENT),
            ),
            Span::raw(" enabled items from "),
            Span::styled(
                format!("{}", st["sources"].as_u64().unwrap_or(0)),
                Style::new().bold().fg(ACCENT),
            ),
            Span::raw(" sources"),
        ]),
        Line::from(vec![
            Span::styled("last sync  ", Style::new().fg(Color::DarkGray)),
            Span::raw(st["last_sync"].as_str().unwrap_or("never").to_owned()),
        ]),
        Line::from(vec![
            Span::styled("fingerprint  ", Style::new().fg(Color::DarkGray)),
            Span::raw(s(&st["fingerprint"]).to_owned()),
        ]),
        if conflicts {
            Line::from(Span::styled(
                "conflicts: see Catalog (shadowed items) or `lo why`",
                Style::new().fg(Color::Red),
            ))
        } else {
            Line::from(Span::styled("no conflicts", Style::new().fg(Color::Green)))
        },
    ];
    f.render_widget(Paragraph::new(text).block(block("Status")), summary);
    let rows: Vec<Row> = st["pending"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            Row::new(vec![
                Cell::from(Span::styled(
                    s(&p["source"]).to_owned(),
                    Style::new().bold(),
                )),
                Cell::from(format!(
                    "{} → {}",
                    p["from"]
                        .as_str()
                        .map_or("(new)".to_owned(), |c| c.chars().take(8).collect()),
                    s(&p["to"]).chars().take(8).collect::<String>()
                )),
                Cell::from(Span::styled(
                    s(&p["reason"]).to_owned(),
                    Style::new().fg(Color::Yellow),
                )),
            ])
        })
        .collect();
    let n = rows.len();
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Length(20),
            Constraint::Min(10),
        ],
    )
    .block(block(&format!("Waiting for review ({n})")))
    .row_highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut ts = TableState::default().with_selected((n > 0).then_some(app.pending_sel));
    f.render_stateful_widget(table, pending, &mut ts);
}

fn form(f: &mut Frame, app: &App) {
    let Some(form) = &app.form else { return };
    let height = form.fields.len() as u16 + 6;
    let area = popup(f.area(), 72, height);
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value, required)) in form.fields.iter().enumerate() {
        let focused = form.focus == i;
        lines.push(Line::from(vec![
            Span::styled(if focused { "▶ " } else { "  " }, Style::new().fg(ACCENT)),
            Span::styled(
                format!("{label:<14}"),
                Style::new().fg(if *required { Color::White } else { Color::Gray }),
            ),
            Span::styled(
                format!("{value}{}", if focused { "▏" } else { "" }),
                if focused {
                    Style::new().fg(ACCENT)
                } else {
                    Style::new()
                },
            ),
        ]));
    }
    let focused = form.focus == form.fields.len();
    lines.push(Line::from(vec![
        Span::styled(if focused { "▶ " } else { "  " }, Style::new().fg(ACCENT)),
        Span::raw(format!("{:<14}", "add to source")),
        Span::styled(
            format!(
                "◀ {} ▶",
                form.dests.get(form.dest).map_or("(none)", String::as_str)
            ),
            if focused {
                Style::new().fg(ACCENT)
            } else {
                Style::new()
            },
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ move · type to edit · ←→ source · enter next / create · esc cancel",
        Style::new().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(block(&format!("Use {}", s(&form.template["id"])))),
        area,
    );
}

fn help(f: &mut Frame) {
    let area = popup(f.area(), 66, 18);
    f.render_widget(Clear, area);
    let rows = [
        ("1-5, tab", "switch view"),
        ("↑ ↓ / j", "move"),
        ("/", "search (catalog)"),
        (
            "s k o r t",
            "cycle a filter: state, kind, source, role, tag",
        ),
        ("c", "clear filters"),
        ("a", "include groups you're not in (catalog)"),
        (
            "space / e",
            "enable, disable, prefer, or join the item's group",
        ),
        ("w / enter", "why this version (catalog)"),
        ("n", "preview a source, then y to subscribe (sources)"),
        ("enter", "use a template · join/leave a group · approve"),
        ("S", "sync now"),
        ("R", "refresh"),
        ("q, ctrl-c", "quit"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::new().fg(ACCENT).bold()),
                Span::raw(*v),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(block("Keys (any key closes)")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::tui::app::Key;
    use crate::tui::app::tests::app;

    fn screen(app: &App) -> String {
        let mut t = Terminal::new(TestBackend::new(110, 30)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        t.backend().to_string()
    }

    #[test]
    fn every_view_renders() {
        let (mut a, _) = app();
        a.on_key(Key::Char('w'));
        insta::assert_snapshot!("catalog", screen(&a));
        a.on_key(Key::Char('k'));
        insta::assert_snapshot!("catalog_filtered", screen(&a));
        a.on_key(Key::Char('2'));
        insta::assert_snapshot!("sources", screen(&a));
        a.on_key(Key::Char('n'));
        for c in "/r/team".chars() {
            a.on_key(Key::Char(c));
        }
        a.on_key(Key::Enter);
        insta::assert_snapshot!("sources_preview", screen(&a));
        a.on_key(Key::Esc);
        a.on_key(Key::Char('3'));
        a.on_key(Key::Enter);
        insta::assert_snapshot!("template_form", screen(&a));
        a.on_key(Key::Esc);
        a.on_key(Key::Char('4'));
        insta::assert_snapshot!("groups", screen(&a));
        a.on_key(Key::Char('5'));
        insta::assert_snapshot!("status", screen(&a));
        a.on_key(Key::Char('?'));
        insta::assert_snapshot!("help", screen(&a));
    }
}
