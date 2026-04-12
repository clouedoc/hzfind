This file provides guidance to AI coding agents like Claude Code (claude.ai/code), Cursor AI, Codex, Gemini CLI, GitHub Copilot, and other AI coding assistants when working with code in this repository.

## Project

hzfind — a Rust CLI/TUI that finds the best deals on the Hetzner Server Auction. It matches auction CPUs against a bundled PassMark benchmark database and computes value-per-euro metrics (CPU score/€, RAM/€, storage/€).

## Commands

```bash
cargo check                  # type-check
cargo clippy --all           # lint (do not suppress warnings)
cargo test                   # run all tests
cargo run -- explore          # launch the interactive TUI
cargo run -- list             # JSON output of all servers
cargo run -- list --sort cpu --top 10  # sort + limit
cargo run -- list-stats       # aggregated auction stats
```

Always run in this order after changes: `cargo check` → `cargo clippy --all` → `cargo test`. Fix issues at each step before moving to the next.

## Architecture

Single-crate binary (`src/main.rs` → `hzfind`). Modules:

- **`lib.rs`** — re-exports `hetzner_auction` and `passmark` as the public API.
- **`hetzner_auction.rs`** — fetches and parses the live Hetzner auction JSON. `HetznerAuction` is the core data struct.
- **`passmark.rs`** — parses the bundled `assets/passmark.json` (embedded via `include_str!`) into a `LazyLock` static (`PASSMARK_SCORES`). CPU name normalization strips `@ frequency` suffixes and standardizes Xeon version strings. Provides `HetznerAuction::cpu_passmark_score()` for matching.
- **`list.rs`** — `build_list()` enriches auction data with PassMark scores into `ListItem` structs with per-euro metrics. `sort_items()` sorts by a `SortField` enum (cpu/ram/storage).
- **`list_stats.rs`** — aggregates match/unmatch stats from auction data.
- **`tui.rs`** — the ratatui-based TUI (table view, detail view, sort picker, VAT dialog). Uses background tokio tasks for live data refresh. Keybindings: j/k or arrows to navigate, s for sort, Enter for detail, v to toggle VAT, t for VAT rate, l for live mode, o to open in browser, q/Esc to quit/back.

Data flow: `fetch_auctions()` → `build_list()` → enriched `ListItem`s consumed by TUI or JSON output.

## Key Structs

### `HetznerAuction` (`src/hetzner_auction.rs`)

Core data struct representing a single Hetzner auction server. Deserialized from the live auction API JSON.

```rust
pub struct HetznerAuction {
    pub id: u32,
    pub cpu: String,
    pub cpu_count: u32,
    pub ram_size: u32,
    pub hdd_size: u32,
    pub hdd_count: u32,
    pub price: f64,           // monthly, VAT excluded, EUR
    pub setup_price: f64,
    pub hourly_price: f64,
    pub ip_price: IpPrice,
    pub datacenter: String,
    pub fixed_price: bool,
    pub next_reduce: u64,
    pub next_reduce_timestamp: Option<u64>,
    pub traffic: String,
    pub bandwidth: u32,
    pub is_ecc: bool,
    pub is_highio: bool,
    pub specials: Vec<String>,
    pub description: Vec<String>,
    pub information: Vec<String>,
    pub dist: Vec<String>,
}
```

### `IpPrice` (`src/hetzner_auction.rs`)

Nested pricing for additional IP addresses.

```rust
pub struct IpPrice {
    pub monthly: f64,
    pub hourly: f64,
}
```

### `ListItem` (`src/list.rs`)

Enriched auction entry with PassMark scores and per-euro value metrics. Produced by `build_list()`.

```rust
pub struct ListItem {
    pub hz_auction_id: u32,
    pub cpu_name: String,
    pub cpu_count: u32,
    pub ram_size_gb: u32,
    pub total_storage_gb: u32,
    pub total_cores: Option<u32>,
    pub p_cores: Option<u32>,
    pub e_cores: Option<u32>,
    pub individual_cpu_score: Option<u32>,
    pub total_cpu_score: Option<u32>,
    pub price_monthly_eur: f64,       // base + IP price, no VAT
    pub cpu_score_per_eur: Option<f64>,
    pub storage_gb_per_eur: f64,
    pub ram_gb_per_eur: f64,
    pub hz_datacenter_location: String,
}
```

### `PassmarkScore` (`src/passmark.rs`)

A single CPU entry from the embedded PassMark database.

```rust
pub struct PassmarkScore {
    pub name: String,
    pub cpumark: u32,
    pub p_threads: u32,
    pub e_threads: u32,
    pub cores: u32,
    pub p_cores: u32,
    pub e_cores: u32,
}
```

### `SortField` (`src/list.rs`)

Enum for sorting `ListItem`s.

```rust
pub enum SortField {
    Cpu,
    Storage,
    Ram,
}
```

## Rules

- Load the Rust skill before writing code or before writing a plan. This is *MANDATORY*.
- The PassMark database is embedded at compile time via `include_str!("../assets/passmark.json")`. To refresh it, run `just passmark` (scrapes cpubenchmark.net).
- Prices in `HetznerAuction.price` are VAT-excluded. The TUI adds VAT on top when enabled (default 20%, toggleable with v/t keys).
- The CCX33 cloud baseline constants in `tui.rs` are used for comparison in the detail view.

## Maintenance: Keep AGENTS.md in sync

When a `pub struct` or `pub enum` listed in **Key Structs** above is modified (fields added/removed/renamed/retyped), or a new important public struct/enum is introduced, update this file to reflect the change. Specifically:

1. **Field changes** — update the corresponding struct block in the **Key Structs** section.
2. **New important struct/enum** — add a new subsection with the struct name, source file, one-line description, and the full struct definition in a code block.
3. **Removed struct/enum** — remove its subsection.

After any structural change, run `cargo check` → `cargo clippy --all` → `cargo test` as usual.
