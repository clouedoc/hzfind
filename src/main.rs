use clap::{Parser, Subcommand};

mod list;
mod list_stats;
mod tui;

#[derive(Parser)]
#[command(name = "hzfind", about = "Hetzner Server Finder")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Output computed server data as JSON to stdout
    List {
        /// Sort by a per-eur field (cpu, storage, ram) — descending, best first
        #[arg(long, value_name = "FIELD")]
        sort: Option<list::SortField>,
        /// Return only the top X elements from the list
        #[arg(long, value_name = "N")]
        top: Option<usize>,
    },
    /// Output aggregated stats about the auction data
    ListStats,
    /// Interactive TUI to browse auction servers
    Explore,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Explore) {
        Command::List { sort, top } => {
            let auctions = hzfind::hetzner_auction::fetch_auctions().await?;
            let mut items = list::build_list(&auctions);
            if let Some(field) = sort {
                list::sort_items(&mut items, field);
            }
            let items = match top {
                Some(n) => &items[..n.min(items.len())],
                None => &items,
            };
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        Command::ListStats => {
            let auctions = hzfind::hetzner_auction::fetch_auctions().await?;
            let stats = list_stats::list_stats(&auctions);
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Command::Explore => tui::run().await?,
    }

    Ok(())
}
