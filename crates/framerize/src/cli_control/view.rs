//! `framerize accounts view` — a frame-wise account view (the Prism-style
//! selector): every account as a row, marked active / skipped / unused, with
//! its workspaces' live balances and dead/saturated marks. Up/down (or j/k)
//! moves, Enter expands a row to its workspaces, r refreshes, d marks a row
//! for removal, x removes every marked row (profile kept), q quits.
//!
//! Data source: GET {daemon}/api/accounts/live (per-account workspace marks,
//! 60s ledger cache). One fetch per refresh; the TUI never holds a request.

use super::http::DaemonClient;
use super::Options;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Stdout;
use std::time::Duration;

pub struct ViewArgs {
    pub options: Options,
}

#[derive(Clone)]
struct Row {
    id: String,
    alias: String,
    verdict: String,
    failures: u32,
    funded: usize,
    unfunded: usize,
    resident: Option<String>,
    cooling_secs: Option<u64>,
    workspaces: Vec<(String, Option<f64>, String)>, // (team8, balance, mark)
}

struct App {
    rows: Vec<Row>,
    selected: usize,
    expanded: HashSet<usize>,
    marked: HashSet<usize>,
    status: String,
}

pub async fn run_view(args: ViewArgs) -> Result<(), String> {
    let mut stdout: Stdout = std::io::stdout();
    enable_raw_mode().map_err(|e| e.to_string())?;
    execute!(stdout, EnterAlternateScreen).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let mut app = App {
        rows: Vec::new(),
        selected: 0,
        expanded: HashSet::new(),
        marked: HashSet::new(),
        status: "r refresh · Enter expand · d mark · x remove marked · q quit".into(),
    };
    let mut app_err: Option<String> = None;
    let out = tui_loop(&mut term, &mut app, &args).await;
    let _ = disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen);
    if let Err(e) = out {
        app_err = Some(e);
    }
    if let Some(e) = app_err {
        return Err(e);
    }
    Ok(())
}

async fn fetch_rows(args: &ViewArgs) -> Result<Vec<Row>, String> {
    let client = DaemonClient::new(&args.options.daemon)?;
    let v: Value = client
        .request(reqwest::Method::GET, "/api/accounts/live")?
        .send()
        .await
        .map_err(super::http::transport_error)?
        .json()
        .await
        .map_err(|e| format!("accounts/live json: {e}"))?;
    let mut rows = Vec::new();
    for a in v["accounts"].as_array().cloned().unwrap_or_default() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let cooling_secs = a["cooling_until_ms"]
            .as_u64()
            .map(|u| u.saturating_sub(now_ms) / 1000);
        rows.push(Row {
            id: a["id"].as_str().unwrap_or("").to_string(),
            alias: a["alias"].as_str().unwrap_or("?").to_string(),
            verdict: a["verdict"].as_str().unwrap_or("?").to_string(),
            failures: a["consecutive_failures"].as_u64().unwrap_or(0) as u32,
            funded: a["funded_workspaces"].as_u64().unwrap_or(0) as usize,
            unfunded: a["unfunded_workspaces"].as_u64().unwrap_or(0) as usize,
            resident: a["resident"].as_str().map(|s| s.to_string()),
            cooling_secs,
            workspaces: a["workspaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|w| {
                    (
                        w["team"].as_str().unwrap_or("?")[..8.min(w["team"].as_str().unwrap_or("?").len())]
                            .to_string(),
                        w["balance"].as_f64(),
                        w["mark"].as_str().unwrap_or("?").to_string(),
                    )
                })
                .collect(),
        });
    }
    Ok(rows)
}

async fn remove_account(options: &Options, id: &str) -> Result<(), String> {
    let client = DaemonClient::new(&options.daemon)?;
    client
        .request(reqwest::Method::POST, "/api/accounts/remove")?
        .json(&serde_json::json!({"account_id": id, "wipe_profile": false}))
        .send()
        .await
        .map_err(super::http::transport_error)?;
    Ok(())
}

async fn tui_loop(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    args: &ViewArgs,
) -> Result<(), String> {
    app.rows = fetch_rows(args).await?;
    loop {
        draw(term, app);
        if !event::poll(Duration::from_millis(250)).map_err(|e| e.to_string())? {
            continue;
        }
        if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('r') => {
                    app.status = "refreshing...".into();
                    match fetch_rows(args).await {
                        Ok(rows) => {
                            app.rows = rows;
                            app.status = format!("{} accounts", app.rows.len());
                        }
                        Err(e) => app.status = e,
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.selected + 1 < app.rows.len() {
                        app.selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.selected = app.selected.saturating_sub(1);
                }
                KeyCode::Enter => {
                    if !app.expanded.remove(&app.selected) {
                        app.expanded.insert(app.selected);
                    }
                }
                KeyCode::Char('d') => {
                    if !app.marked.remove(&app.selected) {
                        app.marked.insert(app.selected);
                    }
                }
                KeyCode::Char('x') => {
                    if app.marked.is_empty() {
                        app.status = "nothing marked (d marks the selected row)".into();
                        continue;
                    }
                    let ids: Vec<String> = app
                        .marked
                        .iter()
                        .filter_map(|i| app.rows.get(*i))
                        .map(|r| r.id.clone())
                        .collect();
                    let mut ok = 0usize;
                    let mut errs = Vec::new();
                    for id in &ids {
                        match remove_account(&args.options, id).await {
                            Ok(()) => ok += 1,
                            Err(e) => errs.push(e),
                        }
                    }
                    app.marked.clear();
                    app.status = if errs.is_empty() {
                        format!("removed {ok} (profiles kept)")
                    } else {
                        format!("removed {ok}; errors: {}", errs.join(" | "))
                    };
                    app.rows = fetch_rows(args).await.unwrap_or_default();
                    if app.selected >= app.rows.len() {
                        app.selected = app.rows.len().saturating_sub(1);
                    }
                }
                _ => {}
            }
        }
    }
}

fn draw(term: &mut Terminal<CrosstermBackend<Stdout>>, app: &App) {
    let _ = term.draw(|f| {
        let chunks = Layout::default()
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(f.area());
        let mut items: Vec<ListItem> = Vec::new();
        for (i, r) in app.rows.iter().enumerate() {
            let color = match r.verdict.as_str() {
                "active" => Color::Green,
                "skipped" => Color::Yellow,
                _ => Color::DarkGray,
            };
            let mark = if app.marked.contains(&i) { " [MARKED]" } else { "" };
            let cool = r
                .cooling_secs
                .map(|s| format!(" cooling {}s", s))
                .unwrap_or_default();
            let resident = r
                .resident
                .as_deref()
                .map(|s| format!(" {}", s))
                .unwrap_or_default();
            let line = format!(
                "{:<22} {:<8} funded:{} unfunded:{} fail:{}{}{}{}",
                r.alias, r.verdict, r.funded, r.unfunded, r.failures, resident, cool, mark
            );
            items.push(ListItem::new(Line::from(Span::styled(line, Style::default().fg(color)))));
            if app.expanded.contains(&i) {
                for (team, bal, mark) in &r.workspaces {
                    let mc = match mark.as_str() {
                        "funded" => Color::Green,
                        "dead" => Color::Red,
                        "saturated" => Color::Yellow,
                        _ => Color::DarkGray,
                    };
                    let txt = format!(
                        "    └ {} {} ({})",
                        team,
                        bal.map(|b| format!("{b:.0}"))
                            .unwrap_or_else(|| "?".into()),
                        mark
                    );
                    items.push(ListItem::new(Line::from(Span::styled(txt, Style::default().fg(mc)))));
                }
            }
        }
        let list = List::new(items)
            .block(Block::default().title(" accounts — live workspace marks ").borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
        let mut st = ListState::default();
        st.select(Some(app.selected));
        f.render_stateful_widget(list, chunks[0], &mut st);
        f.render_widget(
            Paragraph::new(app.status.clone()).block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );
    });
}
