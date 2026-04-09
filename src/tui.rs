use std::io;
use std::time::Duration;

use num_format::ToFormattedString;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use eyre::Result;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, HighlightSpacing, Padding, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};
use ratatui::Frame;
use ratatui::Terminal;

use crate::list::{build_list, sort_items, ListItem, SortField};
use hzfind::hetzner_auction::{fetch_auctions, HetznerAuction};

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
const C_BETTER: Color = Color::Rgb(130, 220, 130);
const C_WORSE: Color = Color::Rgb(220, 130, 130);

// ── CCX33 baseline (Hetzner Cloud, dedicated, excl. VAT, monthly) ──
const CCX33_PRICE: f64 = 62.99;
const CCX33_CPU_SCORE: f64 = 14694.0;
const CCX33_RAM_GB: f64 = 32.0;
const CCX33_STORAGE_GB: f64 = 240.0;
const CCX33_CORES: f64 = 4.0;
const CCX33_CPU_SCORE_PER_EUR: f64 = CCX33_CPU_SCORE / CCX33_PRICE;
const CCX33_RAM_PER_EUR: f64 = CCX33_RAM_GB / CCX33_PRICE;
const CCX33_STORAGE_PER_EUR: f64 = CCX33_STORAGE_GB / CCX33_PRICE;

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Table,
    SortDialog,
    Detail,
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
}

impl App {
    fn new(mut items: Vec<ListItem>, auctions: Vec<HetznerAuction>) -> Self {
        sort_items(&mut items, SortField::Cpu);
        let mut table_state = TableState::default();
        table_state.select(Some(0));
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
        }
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
            self.selected_auction = self
                .auctions
                .iter()
                .find(|a| a.id == item_clone.hz_auction_id)
                .cloned();
            if self.selected_auction.is_some() {
                self.selected_item_data = Some(item_clone);
                self.detail_scroll = 0;
                self.mode = Mode::Detail;
            }
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
        let auctions = fetch_auctions()
            .await
            .expect("failed to fetch Hetzner auctions");
        let items = build_list(&auctions);
        (items, auctions)
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
                if matches!(event, Event::Resize(_, _)) {}
            }
        }

        if handle.is_finished() {
            break;
        }
    }

    let (items, auctions) = handle.await.expect("task panicked");
    let mut app = App::new(items, auctions);

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        if let Some(Ok(event)) = events.next().await {
            match event {
                Event::Resize(_, _) => continue,
                Event::Key(key) if key.kind != KeyEventKind::Press => continue,
                Event::Key(key) => {
                    let prev_mode = app.mode;
                    match app.mode {
                        Mode::Table => handle_table_key(key, &mut app),
                        Mode::SortDialog => handle_sort_dialog_key(key, &mut app),
                        Mode::Detail => handle_detail_key(key, &mut app),
                    }
                    if prev_mode == Mode::Table && (should_quit(&key) || key.code == event::KeyCode::Esc) {
                        break;
                    }
                }
                _ => {}
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
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('n'))
    {
        let next = state.selected().map(|i| i.saturating_add(1)).unwrap_or(0);
        state.select(Some(next.min(len.saturating_sub(1))));
    }
}

fn move_up(key: &KeyEvent, state: &mut TableState, len: usize) {
    if matches!(key.code, event::KeyCode::Up | event::KeyCode::Char('k'))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('p'))
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

fn handle_detail_key(key: KeyEvent, app: &mut App) {
    let scroll_step = 3;

    if matches!(key.code, event::KeyCode::Down | event::KeyCode::Char('j'))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('n'))
    {
        app.detail_scroll = app.detail_scroll.saturating_add(scroll_step);
    }
    if matches!(key.code, event::KeyCode::Up | event::KeyCode::Char('k'))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('p'))
    {
        app.detail_scroll = app.detail_scroll.saturating_sub(scroll_step);
    }
    if matches!(key.code, event::KeyCode::Char('G')) {
        app.detail_scroll = u16::MAX;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(key.code, event::KeyCode::Char('G'))
    {
        app.detail_scroll = 0;
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
    let loading_text = Line::from(vec![
        Span::styled(
            format!(" Fetching Hetzner auction data{dots} "),
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Block::default().style(Style::default().bg(C_ROW)), f.area());
    let centered = centered_rect(34, 3, f.area());
    f.render_widget(
        ratatui::widgets::Paragraph::new(loading_text)
            .alignment(Alignment::Center),
        centered,
    );
}

fn render(f: &mut Frame, app: &mut App) {
    let bg = match app.mode {
        Mode::Table | Mode::SortDialog => C_ROW,
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
    }
}

fn render_table(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(0),   // table
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
            Style::default()
                .fg(C_ACCENT)
                .add_modifier(Modifier::BOLD),
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
    let header_block = Block::default()
        .style(Style::default().bg(C_HEADER_BG).fg(C_HEADER_FG))
        .padding(Padding::horizontal(1));
    let header_inner = header_block.inner(chunks[0]);
    f.render_widget(header_block, chunks[0]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(header),
        header_inner,
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
        Constraint::Length(7),       // ID
        Constraint::Length(max_cpu_len), // CPU
        Constraint::Length(4),  // CPU#
        Constraint::Length(6),  // Cores
        Constraint::Length(8),  // RAM
        Constraint::Length(10), // Storage
        Constraint::Length(10), // CPU Sc/€
        Constraint::Length(10), // Storage/€
        Constraint::Length(10), // RAM/€
        Constraint::Length(7),  // Price
        Constraint::Length(9),  // DC
    ];

    let active_col = match app.sort {
        SortField::Cpu => 6,
        SortField::Ram => 7,
        SortField::Storage => 8,
    };
    let header_cells: Vec<Cell> = [
        "ID", "CPU", "CPU#", "Cores", "RAM", "Storage", "CPU Sc/€", "RAM/€", "Storage/€", "Price", "DC",
    ]
    .into_iter()
    .enumerate()
    .map(|(col, text)| {
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
                Cell::new(item.hz_auction_id.to_string()),
                Cell::new(item.cpu_name.as_str()),
                Cell::new(item.cpu_count.to_string()),
                Cell::new(
                    item.total_cores
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                Cell::new(format!("{} GB", item.ram_size_gb)),
                Cell::new(storage_str(item.total_storage_gb)),
                Cell::new(
                    item.cpu_score_per_eur
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "—".into()),
                ),
                Cell::new(format!("{:.1}", item.ram_gb_per_eur)),
                Cell::new(format!("{:.1}", item.storage_gb_per_eur)),
                Cell::new(format!("€{:.2}", item.price_monthly_eur)),
                Cell::new(item.hz_datacenter_location.as_str()),
            ];
            let cells: Vec<Cell> = cells
                .into_iter()
                .enumerate()
                .map(|(col, cell)| {
                    if col == active_col {
                        cell.style(active_style)
                    } else {
                        cell
                    }
                })
                .collect();
            let mut row = Row::new(cells).style(row_style);
            if i == selected_idx {
                row = row.style(
                    Style::default()
                        .fg(C_HIGHLIGHT_FG)
                        .bg(C_HIGHLIGHT_BG),
                );
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
        .row_highlight_style(
            Style::default()
                .fg(C_HIGHLIGHT_FG)
                .bg(C_HIGHLIGHT_BG),
        )
        .highlight_spacing(HighlightSpacing::Always);

    let mut scrollbar_state = ScrollbarState::new(app.items.len().saturating_sub(1))
        .position(selected_idx);
    f.render_stateful_widget(table, chunks[1], &mut app.table_state);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(C_DIM)),
        chunks[1],
        &mut scrollbar_state,
    );

    // ── Footer ───────────────────────────────────────────────────────────
    let count = app.items.len();
    let footer_text = format!(
        " {} servers │ ↑↓ navigate │ s sort │ Enter details │ q/Esc quit ",
        count
    );
    let footer_block = Block::default().style(
        Style::default()
            .bg(C_FOOTER_BG)
            .fg(C_FOOTER_FG),
    );
    let footer_inner = footer_block.inner(chunks[2]);
    f.render_widget(footer_block, chunks[2]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(footer_text)
            .alignment(Alignment::Center),
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
                .title_style(
                    Style::default()
                        .fg(C_ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
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

fn render_detail(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Min(0),   // detail content
        Constraint::Length(1), // footer
    ])
    .split(area);

    let Some(ref auction) = app.selected_auction else {
        return;
    };

    let total_storage = auction.hdd_size * auction.hdd_count;
    let section = Style::default().fg(C_SECTION).add_modifier(Modifier::BOLD);
    let label = Style::default().fg(C_DIM).add_modifier(Modifier::BOLD);
    let value = Style::default().fg(C_VALUE);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled(&auction.cpu, Style::default().fg(C_ACCENT).bold()),
            Span::styled(
                format!("  × {}", auction.cpu_count),
                Style::default().fg(C_DIM),
            ),
        ]),
        detail_line(
            "  Cores",
            &match auction.cpu_passmark_score() {
                Some(score) => format!(
                    "{} ({} cores × {} CPUs)",
                    score.cores * auction.cpu_count,
                    score.cores,
                    auction.cpu_count
                ),
                None => "—".to_string(),
            },
            label,
            value,
        ),
        detail_line(
            "  PassMark",
            &match auction.cpu_passmark_score() {
                Some(score) => format!(
                    "{} ({} per CPU × {} CPUs)",
                    format_number(score.cpumark as u64 * auction.cpu_count as u64),
                    format_number(score.cpumark as u64),
                    auction.cpu_count
                ),
                None => "—".to_string(),
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
        detail_line("  ID", &auction.id.to_string(), label, value),
        detail_line("  RAM", &format!("{} GB", auction.ram_size), label, value),
        detail_line(
            "  Storage",
            &format!(
                "{} ({} × {} GB)",
                storage_str(total_storage), auction.hdd_count, auction.hdd_size
            ),
            label,
            value,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Pricing", section),
            Span::styled(" ──────────────────────", label),
        ]),
        detail_line(
            "  Monthly",
            &format!("€{:.2}", auction.price),
            label,
            Style::default().fg(C_PRICE),
        ),
        detail_line(
            "  Setup",
            &format!("€{:.2}", auction.setup_price),
            label,
            value,
        ),
        detail_line(
            "  Hourly",
            &format!("€{:.4}", auction.hourly_price),
            label,
            value,
        ),
        detail_line(
            "  IP (monthly)",
            &format!("€{:.2}", auction.ip_price.monthly),
            label,
            value,
        ),
        detail_line(
            "  Fixed Price",
            if auction.fixed_price { "Yes" } else { "No" },
            label,
            value,
        ),
    ];

    // ── vs CCX33 comparison ──
    if let Some(ref item) = app.selected_item_data {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", value),
            Span::styled(
                format!("vs CCX33 (€{CCX33_PRICE:.2}/mo)"),
                section,
            ),
            Span::styled(" ───────────────", label),
        ]));
        lines.push(comparison_line(
            "Cores",
            item.total_cores.map(|c| c as f64),
            CCX33_CORES,
            label,
            |v| format!("{}", v as u32),
        ));
        lines.push(comparison_line(
            "RAM",
            Some(item.ram_size_gb as f64),
            CCX33_RAM_GB,
            label,
            |v| format!("{} GB", v as u32),
        ));
        lines.push(comparison_line(
            "Storage",
            Some(item.total_storage_gb as f64),
            CCX33_STORAGE_GB,
            label,
            |v| storage_str(v as u32),
        ));
        lines.push(comparison_line(
            "CPU Sc/€",
            item.cpu_score_per_eur,
            CCX33_CPU_SCORE_PER_EUR,
            label,
            |v| format!("{v:.1}"),
        ));
        lines.push(comparison_line(
            "RAM/€",
            Some(item.ram_gb_per_eur),
            CCX33_RAM_PER_EUR,
            label,
            |v| format!("{v:.1}"),
        ));
        lines.push(comparison_line(
            "Storage/€",
            Some(item.storage_gb_per_eur),
            CCX33_STORAGE_PER_EUR,
            label,
            |v| format!("{v:.1}"),
        ));
    }

    let more_lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Location & Network", section),
            Span::styled(" ──────────────────────", label),
        ]),
        detail_line("  Datacenter", &auction.datacenter, label, value),
        detail_line("  Traffic", &auction.traffic, label, value),
        detail_line(
            "  Bandwidth",
            &format!("{} Gbit/s", auction.bandwidth),
            label,
            value,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", value),
            Span::styled("Properties", section),
            Span::styled(" ──────────────────────", label),
        ]),
        detail_line(
            "  ECC RAM",
            if auction.is_ecc { "Yes" } else { "No" },
            label,
            value,
        ),
        detail_line(
            "  High IO",
            if auction.is_highio { "Yes" } else { "No" },
            label,
            value,
        ),
    ];
    lines.extend(more_lines);

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
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::raw(d.as_str()),
            ]));
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
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::raw(i.as_str()),
            ]));
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

    lines.push(Line::from(""));

    let block = Block::default()
        .title(" Server Details ")
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(C_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
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
    let mut scrollbar_state = ScrollbarState::new(content_height)
        .position(app.detail_scroll as usize);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(C_DIM)),
        inner,
        &mut scrollbar_state,
    );

    // ── Footer ───────────────────────────────────────────────────────────
    let footer_block = Block::default().style(
        Style::default().bg(C_FOOTER_BG).fg(C_FOOTER_FG),
    );
    let footer_inner = footer_block.inner(chunks[1]);
    f.render_widget(footer_block, chunks[1]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(
            " ↑↓ scroll     [o] Open in browser     [q/Esc] Back to table ",
        )
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
    ccx33_val: f64,
    label_style: Style,
    fmt: impl Fn(f64) -> String,
) -> Line<'static> {
    let padded_label = format!("    {label:<12}");
    match server_val {
        Some(sv) => {
            let pct = (sv - ccx33_val) / ccx33_val * 100.0;
            let pct_color = if pct >= 0.0 { C_BETTER } else { C_WORSE };
            let sign = if pct >= 0.0 { "+" } else { "" };
            Line::from(vec![
                Span::styled(padded_label, label_style),
                Span::styled(
                    format!("{:>8}", fmt(sv)),
                    Style::default().fg(C_VALUE),
                ),
                Span::styled(" vs ", Style::default().fg(C_DIM)),
                Span::styled(
                    format!("{:<8}", fmt(ccx33_val)),
                    Style::default().fg(C_DIM),
                ),
                Span::styled(
                    format!("({sign}{pct:.1}%)"),
                    Style::default().fg(pct_color).add_modifier(Modifier::BOLD),
                ),
            ])
        }
        None => Line::from(vec![
            Span::styled(padded_label, label_style),
            Span::styled("       —", Style::default().fg(C_DIM)),
        ]),
    }
}

fn detail_line(
    lbl: &str,
    val: &str,
    label_style: Style,
    value_style: Style,
) -> Line<'static> {
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
