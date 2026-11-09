//! `framerize accounts view` — a frame-wise account view (the Prism-style
//! selector): every account as a row, marked active / skipped / unused, with
//! its workspaces' live balances and dead/saturated marks. Up/down (or j/k)
//! moves, Enter expands a row to its workspaces, r refreshes, d marks a row
//! for removal, x removes every marked row (profiles kept), q quits.
//!
//! NEVER blocks the input loop on network: fetches run in spawned tasks over
//! a channel; the frame draws immediately with a loading state.
//! Data source: GET {daemon}/api/accounts/live (per-account workspace marks).

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
    workspaces: Vec<(String, Option<f64>, String)>,
}

struct App {
    rows: Vec<Row>,
    selected: usize,
    expanded: HashSet<usize>,
    marked: HashSet<usize>,
    status: String,
    loading: bool,
}

pub async fn run_view(args: ViewArgs) -> Result<(), String> {
    let mut stdout: Stdout = std::io::stdout();
    enable_raw_mode().map_err(|e| format!("no terminal for the view (run it in one): {e}"))?;
    execute!(stdout, EnterAlternateScreen).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let mut app = App {
        rows: Vec::new(),
        selected: 0,
        expanded: HashSet::new(),
        marked: HashSet::new(),
        status: "loading...".into(),
        loading: true,
    };
    let out = tui_loop(&mut term, &mut app, &args).await;
    let _ = disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen);
    out
}

async fn fetch_rows(daemon: &str) -> Result<Vec<Row>, String> {
    let client = DaemonClient::new(daemon)?;
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
                    let t = w["team"].as_str().unwrap_or("?");
                    (
                        t[..8.min(t.len())].to_string(),
                        w["balance"].as_f64(),
                        w["mark"].as_str().unwrap_or("?").to_string(),
                    )
                })
                .collect(),
        });
    }
    Ok(rows)
}

async fn remove_account(daemon: &str, id: &str) -> Result<(), String> {
    let client = DaemonClient::new(daemon)?;
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
    // Fetches run off the input loop: a channel carries results back.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<Row>, String>>(1);
    let daemon = args.options.daemon.clone();
    {
        let tx0 = tx.clone();
        tokio::spawn(async move {
            let _ = tx0.send(fetch_rows(&daemon).await).await;
        });
    }
    loop {
        draw(term, app);
        // New fetch result? Apply it between key polls.
        while let Ok(res) = rx.try_recv() {
            match res {
                Ok(rows) => {
                    app.status = format!("{} accounts", rows.len());
                    app.rows = rows;
                    app.loading = false;
                }
                Err(e) => {
                    app.status = format!("fetch failed: {e} (r retries)");
                    app.loading = false;
                }
            }
        }
        if !event::poll(Duration::from_millis(120)).map_err(|e| e.to_string())? {
            continue;
        }
        if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('r') => {
                    if app.loading {
                        app.status = "still loading...".into();
                        continue;
                    }
                    app.loading = true;
                    app.status = "refreshing...".into();
                    let daemon = args.options.daemon.clone();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx2.send(fetch_rows(&daemon).await).await;
                    });
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
                        match remove_account(&args.options.daemon, id).await {
                            Ok(()) => ok += 1,
                            Err(e) => errs.push(e),
                        }
                    }
                    app.marked.clear();
                    app.status = if errs.is_empty() {
                        format!("removed {ok} (profiles kept); refreshing...")
                    } else {
                        format!("removed {ok}; errors: {}", errs.join(" | "))
                    };
                    app.loading = true;
                    let daemon = args.options.daemon.clone();
                    let tx3 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx3.send(fetch_rows(&daemon).await).await;
                    });
                }
                _ => {}
            }
        }
    }
}

fn draw<B: ratatui::backend::Backend>(term: &mut Terminal<B>, app: &App) {
    let _ = term.draw(|f| {
        let chunks = Layout::default()
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(f.area());
        let mut items: Vec<ListItem> = Vec::new();
        if app.rows.is_empty() && app.loading {
            items.push(ListItem::new(Line::from(Span::styled(
                " loading accounts (first live ledger read)...",
                Style::default().fg(Color::Cyan),
            ))));
        }
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
            Paragraph::new(format!("{} · r refresh · Enter expand · d mark · x remove · q quit", app.status))
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        Row {
            id: "id-1".into(),
            alias: "acct-a".into(),
            verdict: "active".into(),
            failures: 0,
            funded: 2,
            unfunded: 0,
            resident: None,
            cooling_secs: None,
            workspaces: vec![("team-1aa".into(), Some(500.0), "funded".into())],
        }
    }

    #[test]
    fn draws_loading_then_rows_and_expansion() {
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut term = Terminal::new(backend).unwrap();
        let mut app = App {
            rows: vec![],
            selected: 0,
            expanded: HashSet::new(),
            marked: HashSet::new(),
            status: "loading...".into(),
            loading: true,
        };
        draw(&mut term, &app);
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("accounts"), "title missing");
        assert!(text.contains("loading"), "loading state missing");
        assert!(text.contains("q quit"), "key help missing");

        app.rows = vec![row()];
        app.loading = false;
        app.status = "1 accounts".into();
        draw(&mut term, &app);
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("acct-a"), "account row missing: {text}");
        assert!(text.contains("funded:2"), "counts missing");
        assert!(!text.contains("team-1aa"), "workspaces hidden until Enter");

        app.expanded.insert(0);
        draw(&mut term, &app);
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("team-1aa"), "expanded workspace missing");
        assert!(text.contains("500"), "balance missing");
    }

    #[test]
    fn marked_row_shows_marker() {
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut term = Terminal::new(backend).unwrap();
        let mut app = App {
            rows: vec![row()],
            selected: 0,
            expanded: HashSet::new(),
            marked: HashSet::from([0]),
            status: String::new(),
            loading: false,
        };
        draw(&mut term, &app);
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("MARKED"), "marker missing");
        let _ = &mut app;
    }
}
