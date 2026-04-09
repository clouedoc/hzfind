use serde::Deserialize;

use eyre::Result;

const HETZNER_AUCTION_URL: &str =
    "https://www.hetzner.com/_resources/app/data/app/live_data_sb_EUR.json";

#[derive(Debug, Deserialize, Clone)]
pub struct HetznerAuction {
    pub id: u32,
    pub cpu: String,
    pub cpu_count: u32,
    pub ram_size: u32,
    pub hdd_size: u32,
    pub hdd_count: u32,
    /// Monthly price, VAT excluded, in euro.
    pub price: f64,
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

#[derive(Debug, Deserialize, Clone)]
pub struct IpPrice {
    pub monthly: f64,
    pub hourly: f64,
}

/// Intermediate JSON shape matching the raw API response.
#[derive(Deserialize)]
struct RawHetznerAuction {
    id: u32,
    cpu: String,
    cpu_count: u32,
    ram_size: u32,
    hdd_size: u32,
    hdd_count: u32,
    price: f64,
    setup_price: f64,
    hourly_price: f64,
    ip_price: RawIpPrice,
    datacenter: String,
    fixed_price: bool,
    next_reduce: u64,
    next_reduce_timestamp: u64,
    traffic: String,
    bandwidth: u32,
    is_ecc: bool,
    is_highio: bool,
    specials: Vec<String>,
    description: Vec<String>,
    information: Vec<String>,
    dist: Vec<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct RawIpPrice {
    Monthly: f64,
    Hourly: f64,
}

pub async fn fetch_auctions() -> Result<Vec<HetznerAuction>> {
    let json = reqwest::get(HETZNER_AUCTION_URL).await?.text().await?;
    parse_auctions(&json)
}

fn parse_auctions(json: &str) -> Result<Vec<HetznerAuction>> {
    #[derive(Deserialize)]
    struct Root {
        server: Vec<RawHetznerAuction>,
    }

    let root: Root = serde_json::from_str(json)?;
    let auctions: Vec<HetznerAuction> = root
        .server
        .into_iter()
        .map(|raw| HetznerAuction {
            id: raw.id,
            cpu: raw.cpu,
            cpu_count: raw.cpu_count,
            ram_size: raw.ram_size,
            hdd_size: raw.hdd_size,
            hdd_count: raw.hdd_count,
            price: raw.price,
            setup_price: raw.setup_price,
            hourly_price: raw.hourly_price,
            ip_price: IpPrice {
                monthly: raw.ip_price.Monthly,
                hourly: raw.ip_price.Hourly,
            },
            datacenter: raw.datacenter,
            fixed_price: raw.fixed_price,
            next_reduce: raw.next_reduce,
            next_reduce_timestamp: if raw.next_reduce_timestamp == 0 {
                None
            } else {
                Some(raw.next_reduce_timestamp)
            },
            traffic: raw.traffic,
            bandwidth: raw.bandwidth,
            is_ecc: raw.is_ecc,
            is_highio: raw.is_highio,
            specials: raw.specials,
            description: raw.description,
            information: raw.information,
            dist: raw.dist,
        })
        .collect();
    Ok(auctions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_auctions_successfully() {
        let auctions = fetch_auctions()
            .await
            .expect("fetch should succeed");
        assert!(!auctions.is_empty());
        let first = &auctions[0];
        assert!(!first.cpu.is_empty());
        assert!(first.cpu_count > 0);
    }
}
