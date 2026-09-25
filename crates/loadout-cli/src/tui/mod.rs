//! `lo tui`: a terminal UI over the same commands as the
//! CLI and the web UI: browse and filter the catalog, toggle items, see
//! sources and their upstreams, preview and subscribe, use templates, join
//! groups, review pending changes.

pub mod app;
pub mod view;

use std::io::IsTerminal;

use anyhow::{Result, bail};
use clap::Args;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::exit;
use app::{App, Key, Loadout};

#[derive(Debug, Args)]
pub struct TuiArgs {}

/// Runs commands with this `lo` binary, like `lo ui`.
struct Cli<'a> {
    ctx: &'a Ctx,
}

impl Loadout for Cli<'_> {
    fn run(&self, args: &[&str]) -> (i32, Value) {
        crate::ui::run_loadout(self.ctx, args)
    }

    fn sources(&self) -> Value {
        crate::ui::sources_json(self.ctx).unwrap_or(Value::Null)
    }
}

pub fn run(ctx: &Ctx, _args: TuiArgs) -> Result<u8> {
    if ctx.json || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("`lo tui` needs a terminal; use the other commands (with --json) in scripts");
    }
    let mut app = App::new(Box::new(Cli { ctx }));
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            terminal.draw(|f| view::draw(f, &app))?;
            if let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
                && let Some(key) = map_key(k.code, k.modifiers)
            {
                if matches!(key, Key::Char('S'))
                    || (matches!(key, Key::Char('y')) && app.preview.is_some())
                {
                    app.message = Some(("Working…".to_owned(), false));
                    terminal.draw(|f| view::draw(f, &app))?;
                }
                app.on_key(key);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result?;
    Ok(exit::OK)
}

fn map_key(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    Some(match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => Key::CtrlC,
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        _ => return None,
    })
}
