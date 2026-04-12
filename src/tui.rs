use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use num_format::ToFormattedString;

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use eyre::Result;
use futures::StreamExt;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, HighlightSpacing, Padding, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
};

use tokio::sync::RwLock;

use crate::list::{ListItem, SortField, build_list, sort_items};
use hzfind::hetzner_auction::{HetznerAuction, fetch_auctions};
use hzfind::hetzner_cloud::HETZNER_CLOUD_SERVERS;

// ── Colors ───────────────────────────────────────────────────────────────────

const C_HEADER_BG: Color = Color::Rgb(30, 30, 50);
const C_HEADER_FG: Color = Color::Rgb(180, 200, 255);
const C_FOOTER_BG: Color = Color::Rgb(30, 30, 50);
const C_FOOTER_FG: Color = Color::Rgb(120, 120, 150);
const C_ACCENT: Color = Color::Rgb(100, 160, 255);
const C_HIGHLIGHT_BG: Color = Color::Rgb(50, 55, 80);
const C_HIGHLIGHT_FG: Color = Color::Rgb(230, 235, 255);
const C_ROW_ALT: Color = Color::Rgb(35, 37, 55);
const C_ROW: Color = Color::Rgb(25, 27, 40);
const C_DIM: Color = Color::Rgb(90, 95, 120);
const C_VALUE: Color = Color::Rgb(200, 210, 240);
const C_SECTION: Color = Color::Rgb(100, 160, 255);
const C_DIALOG_BG: Color = Color::Rgb(35, 37, 55);
const C_DIALOG_BORDER: Color = Color::Rgb(80, 90, 140);
const C_DIALOG_HIGHLIGHT_BG: Color = Color::Rgb(50, 55, 80);
const C_DIALOG_HIGHLIGHT_FG: Color = Color::Rgb(100, 160, 255);
const C_PRICE: Color = Color::Rgb(130, 220, 130);
const C_VAT_BADGE: Color = Color::Rgb(255, 200, 80);

const DEFAULT_VAT_RATE: f64 = 20.0;
const C_BETTER: Color = Color::Rgb(130, 220, 130);
const C_WORSE: Color = Color::Rgb(220, 130, 130);

// ── Cloud baseline helper ──
/// Returns the first cloud server from the embedded JSON (currently CCX33).
fn cloud_baseline() -> &'static hzfind::hetzner_cloud::HetznerCloudServer {
    HETZNER_CLOUD_SERVERS
        .first()
        .expect("assets/hetzner_cloud.json must contain at least one server")
}

// ── Shared data (background-fetch → render bridge) ─────────────────────────

struct SharedData {
    items: Vec<ListItem>,
    auctions: Vec<HetznerAuction>,
    last_fetched: Instant,
}

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Table,
    SortDialog,
    Detail,
    VatDialog,
}

struct App {
    items: Vec<ListItem>,
    auctions: Vec<HetznerAuction>,
    sort: SortField,
    mode: Mode,
    table_state: TableState,
    sort_state: TableState,
    selected_auction: Option<HetznerAuction>,
    selected_item_data: Option<ListItem>,
    sort_names: Vec<&'static str>,
    detail_scroll: u16,
    vat_enabled: bool,
    vat_rate: f64,
    vat_input: String,
    vat_dialog_parent: Mode,
    live: bool,
    last_fetched: Instant,
    last_fetch_failed: bool,
    shared: Arc<RwLock<Arc<SharedData>>>,
    current_snapshot: Arc<SharedData>,
    fetching: Arc<AtomicBool>,
    fetch_failed_flag: Arc<AtomicBool>,
}

impl App {
    fn new(mut items: Vec<ListItem>, auctions: Vec<HetznerAuction>) -> Self {
        sort_items(&mut items, SortField::Cpu);
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        let last_fetched = Instant::now();
        let snapshot = Arc::new(SharedData {
            items: items.clone(),
            auctions: auctions.clone(),
            last_fetched,
        });
        let shared = Arc::new(RwLock::new(snapshot.clone()));
        Self {
            items,
            auctions,
            sort: SortField::Cpu,
            mode: Mode::Table,
            table_state,
            sort_state: TableState::default().with_selected(0),
            selected_auction: None,
            selected_item_data: None,
            sort_names: vec!["CPU score/€", "RAM/€", "Storage/€"],
            detail_scroll: 0,
            vat_enabled: true,
            vat_rate: DEFAULT_VAT_RATE,
            vat_input: String::new(),
            vat_dialog_parent: Mode::Table,
            live: false,
            last_fetched,
            last_fetch_failed: false,
            shared,
            current_snapshot: snapshot,
            fetching: Arc::new(AtomicBool::new(false)),
            fetch_failed_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Pick up new data from the background fetcher, if available.
    fn sync_data(&mut self) {
        if self.fetch_failed_flag.swap(false, Ordering::Relaxed) {
            self.last_fetch_failed = true;
        }
        if let Ok(guard) = self.shared.try_read()
            && !Arc::ptr_eq(&self.current_snapshot, &guard)
        {
            self.current_snapshot = Arc::clone(&guard);
            let selected_id = self.table_state.selected();
            let mut new_items = self.current_snapshot.items.clone();
            sort_items(&mut new_items, self.sort);
            self.items = new_items;
            self.auctions = self.current_snapshot.auctions.clone();
            self.table_state
                .select(selected_id.map(|i| i.min(self.items.len().saturating_sub(1))));
            self.last_fetched = self.current_snapshot.last_fetched;
            self.last_fetch_failed = false;
        }
    }

    /// Spawn a non-blocking background fetch (no-op if one is already running).
    fn start_fetch(&self) {
        if self.fetching.swap(true, Ordering::Relaxed) {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let fetching = Arc::clone(&self.fetching);
        let fetch_failed_flag = Arc::clone(&self.fetch_failed_flag);
        let sort = self.sort;
        tokio::spawn(async move {
            let result = async {
                let auctions = fetch_auctions().await?;
                let mut items = build_list(&auctions);
                sort_items(&mut items, sort);
                eyre::Ok((items, auctions))
            }
            .await;
            match result {
                Ok((items, auctions)) => {
                    let snapshot = Arc::new(SharedData {
                        items,
                        auctions,
                        last_fetched: Instant::now(),
                    });
                    let mut guard = shared.write().await;
                    *guard = snapshot;
                }
                Err(_) => {
                    fetch_failed_flag.store(true, Ordering::Relaxed);
                }
            }
            fetching.store(false, Ordering::Relaxed);
        });
    }

    fn data_age(&self) -> Duration {
        self.last_fetched.elapsed()
    }

    fn apply_sort(&mut self) {
        sort_items(&mut self.items, self.sort);
        self.table_state.select(Some(0));
    }

    fn selected_item(&self) -> Option<&ListItem> {
        self.table_state.selected().and_then(|i| self.items.get(i))
    }

    fn open_detail(&mut self) {
        if let Some(item) = self.selected_item() {
            let item_clone = item.clone();
            self.selected_auction = match item_clone.id {
                crate::list::ListItemId::HetznerAuctions(auction_id) => {
                    self.auctions.iter().find(|a| a.id == auction_id).cloned()
                }
                _ => None,
            };
            self.selected_item_data = Some(item_clone);
            self.detail_scroll = 0;
            self.mode = Mode::Detail;
        }
    }
}

// ── Terminal setup / teardown ───────────────────────────────────────────────

fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

fn restore_terminal() -> Result<()> {
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

// ── Main loop (async) ───────────────────────────────────────────────────────

pub async fn run() -> Result<()> {
    let mut terminal = init_terminal()?;
    let mut events = crossterm::event::EventStream::new();

    // Spawn data fetch so we can animate the loading screen
    let handle = tokio::spawn(async move {
        let auctions = fetch_auctions().await?;
        let items = build_list(&auctions);
        eyre::Ok((items, auctions))
    });

    // Animated loading screen
    let mut dot_frame: usize = 0;
    let sleep = tokio::time::sleep(Duration::from_millis(350));
    tokio::pin!(sleep);

    loop {
        terminal.draw(|f| render_loading(f, dot_frame))?;

        tokio::select! {
            _ = &mut sleep => {
                dot_frame = (dot_frame + 1) % 4;
                sleep.as_mut().reset(
                    tokio::time::Instant::now() + Duration::from_millis(350),
                );
            }
            Some(Ok(event)) = events.next() => {
                if let Event::Key(key) = event
                    && key.kind == KeyEventKind::Press
                    && (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == event::KeyCode::Char('c')
                        || key.code == event::KeyCode::Char('q'))
                {
                    handle.abort();
                    restore_terminal()?;
                    return Ok(());
                }
            }
        }

        if handle.is_finished() {
            break;
        }
    }

    let data_result = handle
        .await
        .map_err(|e| eyre::eyre!("Task panicked: {e}"))
        .and_then(|r| r);

    let (items, auctions) = match data_result {
        Ok(data) => data,
        Err(err) => {
            terminal.draw(|f| render_error_screen(f, &err))?;
            loop {
                if let Some(Ok(Event::Key(_))) = events.next().await {
                    break;
                }
            }
            restore_terminal()?;
            return Err(err);
        }
    };

    let mut app = App::new(items, auctions);
    let mut refresh_interval = tokio::time::interval(Duration::from_secs(10));
    refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut render_tick = tokio::time::interval(Duration::from_secs(1));
    app.start_fetch();

    loop {
        app.sync_data();
        terminal.draw(|f| render(f, &mut app))?;

        tokio::select! {
            _ = render_tick.tick() => {}
            _ = refresh_interval.tick(), if app.live => {
                app.start_fetch();
            }
            Some(Ok(event)) = events.next() => {
                match event {
                    Event::Resize(_, _) => continue,
                    Event::Key(key) if key.kind != KeyEventKind::Press => continue,
                    Event::Key(key) => {
                        let prev_mode = app.mode;
                        match app.mode {
                            Mode::Table => handle_table_key(key, &mut app),
                            Mode::SortDialog => handle_sort_dialog_key(key, &mut app),
                            Mode::Detail => handle_detail_key(key, &mut app),
                            Mode::VatDialog => handle_vat_dialog_key(key, &mut app),
                        }
                        if prev_mode == Mode::Table && (should_quit(&key) || key.code == event::KeyCode::Esc) {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    restore_terminal()?;
    Ok(())
}

fn should_quit(key: &KeyEvent) -> bool {
    matches!(key.code, event::KeyCode::Char('q'))
}

fn should_close(key: &KeyEvent) -> bool {
    matches!(key.code, event::KeyCode::Esc | event::KeyCode::Char('q'))
}

fn move_down(key: &KeyEvent, state: &mut TableState, len: usize) {
    if matches!(key.code, event::KeyCode::Down | event::KeyCode::Char('j'))
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == event::KeyCode::Char('n'))
    {
        let next = state.selected().map(|i| i.saturating_add(1)).unwrap_or(0);
        state.select(Some(next.min(len.saturating_sub(1))));
    }
}

fn move_up(key: &KeyEvent, state: &mut TableState, len: usize) {
    if matches!(key.code, event::KeyCode::Up | event::KeyCode::Char('k'))
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == event::KeyCode::Char('p'))
    {
        let next = state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
        state.select(Some(next));
        if state.selected().is_none() && len > 0 {
            state.select(Some(0));
        }
    }
}

fn handle_table_key(key: KeyEvent, app: &mut App) {
    move_down(&key, &mut app.table_state, app.items.len());
    move_up(&key, &mut app.table_state, app.items.len());

    if key.code == event::KeyCode::Char('s') {
        app.sort_state.select(Some(match app.sort {
            SortField::Cpu => 0,
            SortField::Ram => 1,
            SortField::Storage => 2,
        }));
        app.mode = Mode::SortDialog;
    }

    if key.code == event::KeyCode::Char('v') {
        app.vat_enabled = !app.vat_enabled;
    }

    if key.code == event::KeyCode::Char('l') {
        app.live = !app.live;
        if app.live {
            app.start_fetch();
        }
    }

    if key.code == event::KeyCode::Char('t') && app.vat_enabled {
        app.vat_input = format!("{:.0}", app.vat_rate);
        app.vat_dialog_parent = app.mode;
        app.mode = Mode::VatDialog;
    }

    if key.code == event::KeyCode::Enter {
        app.open_detail();
    }
}

fn handle_sort_dialog_key(key: KeyEvent, app: &mut App) {
    move_down(&key, &mut app.sort_state, app.sort_names.len());
    move_up(&key, &mut app.sort_state, app.sort_names.len());

    if key.code == event::KeyCode::Char('c') {
        app.sort = SortField::Cpu;
        app.apply_sort();
        app.mode = Mode::Table;
        return;
    }

    if key.code == event::KeyCode::Char('r') {
        app.sort = SortField::Ram;
        app.apply_sort();
        app.mode = Mode::Table;
        return;
    }

    if key.code == event::KeyCode::Char('s') {
        app.sort = SortField::Storage;
        app.apply_sort();
        app.mode = Mode::Table;
        return;
    }

    if key.code == event::KeyCode::Enter {
        if let Some(i) = app.sort_state.selected() {
            app.sort = match i {
                0 => SortField::Cpu,
                1 => SortField::Ram,
                2 => SortField::Storage,
                _ => app.sort,
            };
            app.apply_sort();
        }
        app.mode = Mode::Table;
    }

    if should_close(&key) {
        app.mode = Mode::Table;
    }
}

fn handle_vat_dialog_key(key: KeyEvent, app: &mut App) {
    match key.code {
        event::KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
            if app.vat_input.len() < 6 {
                app.vat_input.push(c);
            }
        }
        event::KeyCode::Backspace => {
            app.vat_input.pop();
        }
        event::KeyCode::Enter => {
            if let Ok(rate) = app.vat_input.parse::<f64>() {
                app.vat_rate = rate.clamp(0.0, 999.0);
            }
            app.mode = app.vat_dialog_parent;
        }
        _ if should_close(&key) => {
            app.vat_input = String::new();
            app.mode = app.vat_dialog_parent;
        }
        _ => {}
    }
}

fn handle_detail_key(key: KeyEvent, app: &mut App) {
    let scroll_step = 3;

    if matches!(key.code, event::KeyCode::Down | event::KeyCode::Char('j'))
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == event::KeyCode::Char('n'))
    {
        app.detail_scroll = app.detail_scroll.saturating_add(scroll_step);
    }
    if matches!(key.code, event::KeyCode::Up | event::KeyCode::Char('k'))
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == event::KeyCode::Char('p'))
    {
        app.detail_scroll = app.detail_scroll.saturating_sub(scroll_step);
    }
    if matches!(key.code, event::KeyCode::Char('G')) {
        app.detail_scroll = u16::MAX;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) && matches!(key.code, event::KeyCode::Char('G'))
    {
        app.detail_scroll = 0;
    }

    if key.code == event::KeyCode::Char('v') {
        app.vat_enabled = !app.vat_enabled;
    }

    if key.code == event::KeyCode::Char('t') && app.vat_enabled {
        app.vat_input = format!("{:.0}", app.vat_rate);
        app.vat_dialog_parent = app.mode;
        app.mode = Mode::VatDialog;
    }

    if key.code == event::KeyCode::Char('o')
        && let Some(ref auction) = app.selected_auction
    {
        let cpu = auction.cpu.replace(' ', "+");
        let _ = open::that(format!("https://www.hetzner.com/sb/#search={cpu}"));
    }

    if should_close(&key) {
        app.mode = Mode::Table;
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn render_loading(f: &mut Frame, dot_frame: usize) {
    let dots = ".".repeat(dot_frame);
    let loading_text = Line::from(vec![Span::styled(
        format!(" Fetching Hetzner auction data{dots} "),
        Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
    )]);
    f.render_widget(Block::default().style(Style::default().bg(C_ROW)), f.area());
    let centered = centered_rect(34, 3, f.area());
    f.render_widget(
        ratatui::widgets::Paragraph::new(loading_text).alignment(Alignment::Center),
        centered,
    );
}

fn render_error_screen(f: &mut Frame, error: &eyre::Report) {
    f.render_widget(Block::default().style(Style::default().bg(C_ROW)), f.area());

    let area = centered_rect(64, 13, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Error ")
        .title_alignment(Alignment::Center)
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(180, 60, 60)))
        .style(Style::default().bg(C_DIALOG_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(2), // title
        Constraint::Min(0),    // error chain
        Constraint::Length(1), // spacer
        Constraint::Length(1), // hint
    ])
    .split(inner);

    let title = Line::from(Span::styled(
        "✕  Failed to fetch Hetzner auction data",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(title), chunks[0]);

    let mut error_lines: Vec<Line> = error
        .chain()
        .take(6)
        .map(|err| Line::from(Span::styled(err.to_string(), Style::default().fg(C_VALUE))))
        .collect();
    if error.chain().count() > 6 {
        error_lines.push(Line::from(Span::styled("  …", Style::default().fg(C_DIM))));
    }
    f.render_widget(
        ratatui::widgets::Paragraph::new(error_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );

    let hint = Line::from(Span::styled(
        "Press any key to exit",
        Style::default().fg(C_DIM),
    ));
    f.render_widget(
        ratatui::widgets::Paragraph::new(hint).alignment(Alignment::Center),
        chunks[3],
    );
}

fn render(f: &mut Frame, app: &mut App) {
    let bg = match app.mode {
        Mode::Table | Mode::SortDialog | Mode::VatDialog => C_ROW,
        Mode::Detail => C_DIALOG_BG,
    };
    f.render_widget(Block::default().style(Style::default().bg(bg)), f.area());
    match app.mode {
        Mode::Table => render_table(f, app),
        Mode::SortDialog => {
            render_table(f, app);
            render_sort_dialog(f, app);
        }
        Mode::Detail => render_detail(f, app),
        Mode::VatDialog => {
            match app.vat_dialog_parent {
                Mode::Table => render_table(f, app),
                Mode::Detail => render_detail(f, app),
                _ => render_table(f, app),
            }
            render_vat_dialog(f, app);
        }
    }
}

fn render_table(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(0),    // table
        Constraint::Length(1), // footer
    ])
    .split(area);

    // ── Header ───────────────────────────────────────────────────────────
    let sort_label = match app.sort {
        SortField::Cpu => "cpu score/€",
        SortField::Storage => "storage/€",
        SortField::Ram => "ram/€",
    };
    let header = Line::from(vec![
        Span::styled("⟨", Style::default().fg(C_ACCENT)),
        Span::styled(
            " hzfind ",
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("⟩", Style::default().fg(C_ACCENT)),
        Span::raw("  Hetzner Server Auction Finder"),
        Span::raw("    "),
        Span::styled("Sort: ", Style::default().fg(C_DIM)),
        Span::styled(
            sort_label,
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  [s]", Style::default().fg(C_DIM)),
    ]);
    let header = if app.vat_enabled {
        let mut spans = header.spans;
        spans.push(Span::raw("    "));
        spans.push(Span::styled(
            format!(" VAT {:.0}% ", app.vat_rate),
            Style::default()
                .fg(Color::Black)
                .bg(C_VAT_BADGE)
                .add_modifier(Modifier::BOLD),
        ));
        Line::from(spans)
    } else {
        header
    };

    let header_block = Block::default()
        .style(Style::default().bg(C_HEADER_BG).fg(C_HEADER_FG))
        .padding(Padding::horizontal(1));
    let header_inner = header_block.inner(chunks[0]);
    f.render_widget(header_block, chunks[0]);

    let header_cols = Layout::horizontal([
        Constraint::Min(0),     // left side
        Constraint::Length(28), // right side: age display
    ])
    .split(header_inner);

    f.render_widget(ratatui::widgets::Paragraph::new(header), header_cols[0]);

    // Right-aligned: last fetched age + live indicator
    let age_secs = app.data_age().as_secs();
    let age_str = if age_secs < 60 {
        format!("{}s", age_secs)
    } else {
        format!("{}m {}s", age_secs / 60, age_secs % 60)
    };
    // Right-aligned: last fetched age (only when live mode is active)
    let mut right_spans = Vec::new();
    if app.live {
        if app.last_fetch_failed {
            right_spans.push(Span::styled(
                "last fetch failed ",
                Style::default().fg(Color::Yellow),
            ));
        }
        right_spans.push(Span::styled(
            format!(" last fetched: {age_str} "),
            if age_secs >= 60 {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(C_DIM)
            },
        ));
    }
    f.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right),
        header_cols[1],
    );

    // ── Table ────────────────────────────────────────────────────────────
    let max_cpu_len = app
        .items
        .iter()
        .map(|i| i.cpu_name.as_str().chars().count())
        .max()
        .unwrap_or(0)
        .max(6) as u16;
    let widths = [
        Constraint::Length(12),          // ID
        Constraint::Length(1),           // sep
        Constraint::Length(max_cpu_len), // CPU
        Constraint::Length(1),           // sep
        Constraint::Length(4),           // CPU#
        Constraint::Length(8),           // Cores
        Constraint::Length(8),           // RAM
        Constraint::Length(10),          // Storage
        Constraint::Length(1),           // sep
        Constraint::Length(10),          // CPU Sc/€
        Constraint::Length(10),          // RAM/€
        Constraint::Length(10),          // Storage/€
        Constraint::Length(1),           // sep
        Constraint::Length(10),          // Price
        Constraint::Length(1),           // sep
        Constraint::Length(9),           // DC
    ];

    let active_col = match app.sort {
        SortField::Cpu => 9,
        SortField::Ram => 10,
        SortField::Storage => 11,
    };
    let sep_style = Style::default().fg(C_DIM);
    const SEP_COLS: [usize; 5] = [1, 3, 8, 12, 14];
    let header_cells: Vec<Cell> = [
        "ID",
        "│",
        "CPU",
        "│",
        "CPU#",
        "Cores",
        "RAM",
        "Storage",
        "│",
        "CPU Sc/€",
        "RAM/€",
        "Storage/€",
        "│",
        "Price",
        "│",
        "DC",
    ]
    .into_iter()
    .enumerate()
    .map(|(col, text)| {
        if SEP_COLS.contains(&col) {
            return Cell::new(text).style(sep_style);
        }
        let style = if col == active_col {
            Style::default()
                .fg(C_ACCENT)
                .bg(C_HEADER_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(C_HEADER_FG)
                .bg(C_HEADER_BG)
                .add_modifier(Modifier::BOLD)
        };
        Cell::new(text).style(style)
    })
    .collect();

    let selected_idx = app.table_state.selected().unwrap_or(0);
    let rows: Vec<Row> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_alt = i % 2 == 1;
            let base_bg = if is_alt { C_ROW_ALT } else { C_ROW };
            let row_style = Style::default().fg(C_VALUE).bg(base_bg);
            let active_style = Style::default().fg(C_ACCENT).bg(base_bg).bold();
            let cells = [
                Cell::new(item.id.to_string()),
                Cell::new("│").style(sep_style),
                Cell::new(item.cpu_name.as_str()),
                Cell::new("│").style(sep_style),
                Cell::new(item.cpu_count.to_string()),
                Cell::new(match (item.p_cores, item.e_cores) {
                    (None, _) => "—".into(),
                    (Some(p), Some(e)) if e > 0 => format!("{p}P+{e}E"),
                    (Some(_), _) => item
                        .total_cores
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "—".into()),
                }),
                Cell::new(format!("{} GB", item.ram_size_gb)),
                Cell::new(storage_str(item.total_storage_gb)),
                Cell::new("│").style(sep_style),
                Cell::new(
                    item.cpu_score_per_eur
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "—".into()),
                ),
                Cell::new(format!("{:.1}", item.ram_gb_per_eur)),
                Cell::new(format!("{:.1}", item.storage_gb_per_eur)),
                Cell::new("│").style(sep_style),
                Cell::new(if app.vat_enabled {
                    format!(
                        "€{:.2}",
                        item.price_monthly_eur * (1.0 + app.vat_rate / 100.0)
                    )
                } else {
                    format!("€{:.2}", item.price_monthly_eur)
                }),
                Cell::new("│").style(sep_style),
                Cell::new(item.hz_datacenter_location.as_str()),
            ];
            let cells: Vec<Cell> = cells
                .into_iter()
                .enumerate()
                .map(|(col, cell)| {
                    if SEP_COLS.contains(&col) {
                        return cell;
                    }
                    if col == active_col {
                        cell.style(active_style)
                    } else {
                        cell
                    }
                })
                .collect();
            let mut row = Row::new(cells).style(row_style);
            if i == selected_idx {
                row = row.style(Style::default().fg(C_HIGHLIGHT_FG).bg(C_HIGHLIGHT_BG));
            }
            row
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(
            Row::new(header_cells)
                .style(
                    Style::default()
                        .fg(C_HEADER_FG)
                        .bg(C_HEADER_BG)
                        .add_modifier(Modifier::BOLD),
                )
                .bottom_margin(0),
        )
        .row_highlight_style(Style::default().fg(C_HIGHLIGHT_FG).bg(C_HIGHLIGHT_BG))
        .highlight_spacing(HighlightSpacing::Always);

    let mut scrollbar_state =
        ScrollbarState::new(app.items.len().saturating_sub(1)).position(selected_idx);
    f.render_stateful_widget(table, chunks[1], &mut app.table_state);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Style::default().fg(C_DIM)),
        chunks[1],
        &mut scrollbar_state,
    );

    // ── Footer ───────────────────────────────────────────────────────────
    let count = app.items.len();
    let live_tag = if app.live {
        " │ l disable live mode"
    } else {
        " │ l enable live mode"
    };
    let footer_text = if app.vat_enabled {
        format!(
            " {} servers │ ↑↓ navigate │ s sort │ v toggle VAT │ t VAT rate │ Enter details │{live_tag} │ q/Esc quit ",
            count
        )
    } else {
        format!(
            " {} servers │ ↑↓ navigate │ s sort │ Enter details │{live_tag} │ q/Esc quit ",
            count
        )
    };
    let footer_block = Block::default().style(Style::default().bg(C_FOOTER_BG).fg(C_FOOTER_FG));
    let footer_inner = footer_block.inner(chunks[2]);
    f.render_widget(footer_block, chunks[2]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(footer_text).alignment(Alignment::Center),
        footer_inner,
    );
}

fn render_sort_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(30, 9, f.area());
    f.render_widget(Clear, area);

    let rows: Vec<Row> = app
        .sort_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let shortcut = match i {
                0 => "c",
                1 => "r",
                2 => "s",
                _ => "",
            };
            Row::new([Cell::new(format!("[{shortcut}] {name}"))])
        })
        .collect();

    let table = Table::new(rows, [Constraint::Percentage(100)])
        .header(
            Row::new([Cell::new("Sort by")])
                .style(Style::default().fg(C_ACCENT).bold())
                .bottom_margin(1),
        )
        .block(
            Block::default()
                .title(" Sort Picker ")
                .title_alignment(Alignment::Center)
                .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_DIALOG_BORDER))
                .style(Style::default().bg(C_DIALOG_BG).fg(C_VALUE)),
        )
        .row_highlight_style(
            Style::default()
                .fg(C_DIALOG_HIGHLIGHT_FG)
                .bg(C_DIALOG_HIGHLIGHT_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(table, area, &mut app.sort_state);
}

fn render_vat_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(32, 7, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" VAT Rate ")
        .title_alignment(Alignment::Center)
        .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_DIALOG_BORDER))
        .style(Style::default().bg(C_DIALOG_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    let prompt = Line::from(vec![Span::styled(
        "Enter VAT rate (%): ",
        Style::default().fg(C_DIM),
    )]);
    f.render_widget(ratatui::widgets::Paragraph::new(prompt), lines[0]);

    let input_with_cursor = format!("{}█", app.vat_input);
    f.render_widget(
        ratatui::widgets::Paragraph::new(input_with_cursor).style(Style::default().fg(C_VALUE)),
        lines[1],
    );
}

fn render_detail(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Min(0),    // detail content
        Constraint::Length(1), // footer
    ])
    .split(area);

    let Some(ref item) = app.selected_item_data else {
        return;
    };

    let section = Style::default().fg(C_SECTION).add_modifier(Modifier::BOLD);
    let label = Style::default().fg(C_DIM).add_modifier(Modifier::BOLD);
    let value = Style::default().fg(C_VALUE);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled(&item.cpu_name, Style::default().fg(C_ACCENT).bold()),
            Span::styled(
                format!("  × {}", item.cpu_count),
                Style::default().fg(C_DIM),
            ),
        ]),
        detail_line(
            "  Cores",
            &match (item.total_cores, item.p_cores, item.e_cores) {
                (Some(total), Some(p), Some(e)) if e > 0 => {
                    format!("{total} ({p}P + {e}E)")
                }
                (Some(total), _, _) => format!("{total}"),
                _ => "—".to_string(),
            },
            label,
            value,
        ),
        detail_line(
            "  PassMark",
            &match (item.individual_cpu_score, item.total_cpu_score, item.cpu_count) {
                (Some(indiv), Some(total), count) if count > 1 => format!(
                    "{} ({} per CPU × {count} CPUs)",
                    format_number(total as u64),
                    format_number(indiv as u64),
                ),
                (Some(_), Some(total), 1) => format_number(total as u64),
                _ => "—".to_string(),
            },
            label,
            value,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Server Info", section),
            Span::styled(" ──────────────────────", label),
        ]),
        detail_line("  ID", &item.id.to_string(), label, value),
        detail_line("  RAM", &format!("{} GB", item.ram_size_gb), label, value),
    ];

    let storage_text = if item.total_storage_gb > 0 {
        storage_str(item.total_storage_gb)
    } else {
        "—".to_string()
    };
    lines.push(detail_line(
        "  Storage",
        &storage_text,
        label,
        value,
    ));

    // ── Auction-specific pricing section ──
    if let Some(ref auction) = app.selected_auction {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Pricing", section),
            Span::styled(" ──────────────────────", label),
        ]));
        lines.push(detail_line(
            "  Monthly",
            &if app.vat_enabled {
                format!(
                    "€{:.2} (VAT incl.)",
                    auction.price * (1.0 + app.vat_rate / 100.0)
                )
            } else {
                format!("€{:.2}", auction.price)
            },
            label,
            value,
        ));
        lines.push(detail_line(
            "  Setup",
            &if app.vat_enabled {
                format!(
                    "€{:.2} (VAT incl.)",
                    auction.setup_price * (1.0 + app.vat_rate / 100.0)
                )
            } else {
                format!("€{:.2}", auction.setup_price)
            },
            label,
            value,
        ));
        lines.push(detail_line(
            "  Hourly",
            &if app.vat_enabled {
                format!(
                    "€{:.4} (VAT incl.)",
                    auction.hourly_price * (1.0 + app.vat_rate / 100.0)
                )
            } else {
                format!("€{:.4}", auction.hourly_price)
            },
            label,
            value,
        ));
        lines.push(detail_line(
            "  IP (monthly)",
            &if app.vat_enabled {
                format!(
                    "€{:.2} (VAT incl.)",
                    auction.ip_price.monthly * (1.0 + app.vat_rate / 100.0)
                )
            } else {
                format!("€{:.2}", auction.ip_price.monthly)
            },
            label,
            value,
        ));
        lines.push(detail_line(
            "  Total Monthly",
            &if app.vat_enabled {
                let total =
                    (auction.price + auction.ip_price.monthly) * (1.0 + app.vat_rate / 100.0);
                format!("€{:.2} (VAT incl.)", total)
            } else {
                format!("€{:.2}", auction.price + auction.ip_price.monthly)
            },
            label,
            Style::default().fg(C_PRICE),
        ));
        lines.push(detail_line(
            "  Fixed Price",
            if auction.fixed_price { "Yes" } else { "No" },
            label,
            value,
        ));
    } else {
        // Cloud server — show the single monthly price
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Pricing", section),
            Span::styled(" ──────────────────────", label),
        ]));
        lines.push(detail_line(
            "  Monthly",
            &if app.vat_enabled {
                format!(
                    "€{:.2} (VAT incl.)",
                    item.price_monthly_eur * (1.0 + app.vat_rate / 100.0)
                )
            } else {
                format!("€{:.2}", item.price_monthly_eur)
            },
            label,
            Style::default().fg(C_PRICE),
        ));
    }

    // ── vs cloud baseline comparison ──
    if let Some(ref item) = app.selected_item_data {
        let bl = cloud_baseline();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", value),
            Span::styled(
                format!("vs {} (€{:.2}/mo)", bl.name, bl.price_monthly_eur),
                section,
            ),
            Span::styled(" ───────────────", label),
        ]));
        lines.push(comparison_line(
            "Cores",
            item.total_cores.map(|c| c as f64),
            Some(bl.cores as f64),
            label,
            |v| format!("{}", v as u32),
        ));
        lines.push(comparison_line(
            "RAM",
            Some(item.ram_size_gb as f64),
            Some(bl.ram_gb as f64),
            label,
            |v| format!("{} GB", v as u32),
        ));
        lines.push(comparison_line(
            "Storage",
            Some(item.total_storage_gb as f64),
            Some(bl.storage_gb as f64),
            label,
            |v| storage_str(v as u32),
        ));
        lines.push(comparison_line(
            "CPU Sc/€",
            item.cpu_score_per_eur,
            Some(bl.cpu_score_per_eur()),
            label,
            |v| format!("{v:.1}"),
        ));
        lines.push(comparison_line(
            "RAM/€",
            Some(item.ram_gb_per_eur),
            Some(bl.ram_per_eur()),
            label,
            |v| format!("{v:.1}"),
        ));
        lines.push(comparison_line(
            "Storage/€",
            Some(item.storage_gb_per_eur),
            Some(bl.storage_per_eur()),
            label,
            |v| format!("{v:.1}"),
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", value),
        Span::styled("Location & Network", section),
        Span::styled(" ──────────────────────", label),
    ]));
    lines.push(detail_line("  Datacenter", &item.hz_datacenter_location, label, value));

    if let Some(ref auction) = app.selected_auction {
        lines.push(detail_line("  Traffic", &auction.traffic, label, value));
        lines.push(detail_line(
            "  Bandwidth",
            &format!("{} Gbit/s", auction.bandwidth),
            label,
            value,
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Properties", section),
            Span::styled(" ──────────────────────", label),
        ]));
        lines.push(detail_line(
            "  ECC RAM",
            if auction.is_ecc { "Yes" } else { "No" },
            label,
            value,
        ));
        lines.push(detail_line(
            "  High IO",
            if auction.is_highio { "Yes" } else { "No" },
            label,
            value,
        ));
    }

    if let Some(ref auction) = app.selected_auction {
        if !auction.specials.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", value),
                Span::styled("Specials", section),
                Span::styled(" ──────────────────────", label),
            ]));
            for s in &auction.specials {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::raw(format!("• {s}")),
                ]));
            }
        }

        if !auction.description.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", value),
                Span::styled("Description", section),
                Span::styled(" ──────────────────────", label),
            ]));
            for d in &auction.description {
                lines.push(Line::from(vec![Span::raw("      "), Span::raw(d.as_str())]));
            }
        }

        if !auction.information.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", value),
                Span::styled("Information", section),
                Span::styled(" ──────────────────────", label),
            ]));
            for i in &auction.information {
                lines.push(Line::from(vec![Span::raw("      "), Span::raw(i.as_str())]));
            }
        }

        if !auction.dist.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", value),
                Span::styled("Available Distros", section),
                Span::styled(" ──────────────────────", label),
            ]));
            for d in &auction.dist {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::raw(format!("• {d}")),
                ]));
            }
        }
    }

    lines.push(Line::from(""));

    let block = Block::default()
        .title(" Server Details ")
        .title_alignment(Alignment::Center)
        .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_DIALOG_BORDER))
        .style(Style::default().bg(C_DIALOG_BG));

    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    let content_height = lines.len();
    let visible_height = inner.height as usize;
    let max_scroll = content_height.saturating_sub(visible_height).max(0) as u16;
    app.detail_scroll = app.detail_scroll.min(max_scroll);
    f.render_widget(
        ratatui::widgets::Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0)),
        inner,
    );
    let mut scrollbar_state =
        ScrollbarState::new(content_height).position(app.detail_scroll as usize);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Style::default().fg(C_DIM)),
        inner,
        &mut scrollbar_state,
    );

    // ── Footer ───────────────────────────────────────────────────────────
    let footer_block = Block::default().style(Style::default().bg(C_FOOTER_BG).fg(C_FOOTER_FG));
    let footer_inner = footer_block.inner(chunks[1]);
    f.render_widget(footer_block, chunks[1]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(if app.vat_enabled {
            " ↑↓ scroll │ v toggle VAT │ t VAT rate │ o open browser │ q/Esc back "
        } else {
            " ↑↓ scroll │ v toggle VAT │ o open browser │ q/Esc back "
        })
        .alignment(Alignment::Center),
        footer_inner,
    );
}

fn format_number(n: u64) -> String {
    n.to_formatted_string(&num_format::Locale::en)
}

fn comparison_line(
    label: &str,
    server_val: Option<f64>,
    baseline_val: Option<f64>,
    label_style: Style,
    fmt: impl Fn(f64) -> String,
) -> Line<'static> {
    let padded_label = format!("    {label:<12}");
    match (server_val, baseline_val) {
        (Some(sv), Some(bv)) => {
            let pct = (sv - bv) / bv * 100.0;
            let pct_color = if pct >= 0.0 { C_BETTER } else { C_WORSE };
            let sign = if pct >= 0.0 { "+" } else { "" };
            Line::from(vec![
                Span::styled(padded_label, label_style),
                Span::styled(format!("{:>8}", fmt(sv)), Style::default().fg(C_VALUE)),
                Span::styled(" vs ", Style::default().fg(C_DIM)),
                Span::styled(format!("{:<8}", fmt(bv)), Style::default().fg(C_DIM)),
                Span::styled(
                    format!("({sign}{pct:.1}%)"),
                    Style::default().fg(pct_color).add_modifier(Modifier::BOLD),
                ),
            ])
        }
        _ => Line::from(vec![
            Span::styled(padded_label, label_style),
            Span::styled("       —", Style::default().fg(C_DIM)),
        ]),
    }
}

fn detail_line(lbl: &str, val: &str, label_style: Style, value_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{lbl}: "), label_style),
        Span::styled(val.to_string(), value_style),
    ])
}

fn storage_str(gb: u32) -> String {
    if gb >= 1000 {
        format!("{:.1} TB", gb as f64 / 1024.0)
    } else {
        format!("{gb} GB")
    }
}

fn centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.width.saturating_sub(width).saturating_div(2);
    let y = r.height.saturating_sub(height).saturating_div(2);
    Rect::new(x, y, width.min(r.width), height.min(r.height))
}
