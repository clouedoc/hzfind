use serde::Deserialize;

use eyre::Result;

const HETZNER_AUCTION_URL: &str =
    "https://www.hetzner.com/_resources/app/data/app/live_data_sb.json";

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
    pub next_reduce: i64,
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
#[serde(rename_all = "PascalCase")]
struct RawHetznerAuction {
    id: u32,
    hardware: RawHardware,
    prices: RawPrices,
    #[serde(rename = "IPPrices")]
    ip_prices: RawIpPrices,
    details: RawDetails,
    timer: RawTimer,
}

#[derive(Deserialize)]
struct RawHardware {
    #[serde(rename = "CPU")]
    cpu: RawCpu,
    #[serde(rename = "RAM")]
    ram: RawRam,
    #[serde(rename = "Storage")]
    storage: RawStorage,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawCpu {
    name: String,
    core_count: u32,
}

#[derive(Deserialize)]
struct RawRam {
    #[serde(rename = "Size")]
    size: u32,
    ecc: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawStorage {
    size: u32,
    amount: u32,
}

#[derive(Deserialize)]
struct RawPrices {
    monthly: RawCurrencyPrice,
    hourly: RawCurrencyPrice,
    setup: RawCurrencyPrice,
    fixed: bool,
}

#[derive(Deserialize)]
struct RawCurrencyPrice {
    #[serde(rename = "EUR")]
    eur: f64,
}

#[derive(Deserialize)]
struct RawIpPrices {
    monthly: RawCurrencyPrice,
    hourly: RawCurrencyPrice,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawDetails {
    description: Vec<String>,
    information: Vec<String>,
    specials: Vec<String>,
    traffic: String,
    bandwidth: u32,
    #[serde(rename = "OS")]
    os: Vec<String>,
    datacenter: RawDatacenter,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawDatacenter {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawTimer {
    reduce_next: i64,
    reduce_next_timestamp: u64,
}

pub async fn fetch_auctions() -> Result<Vec<HetznerAuction>> {
    let json = reqwest::get(HETZNER_AUCTION_URL)
        .await?
        .error_for_status()?
        .text()
        .await?;
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
            cpu: raw.hardware.cpu.name,
            cpu_count: raw.hardware.cpu.core_count,
            ram_size: raw.hardware.ram.size,
            hdd_size: raw.hardware.storage.size,
            hdd_count: raw.hardware.storage.amount,
            price: raw.prices.monthly.eur,
            setup_price: raw.prices.setup.eur,
            hourly_price: raw.prices.hourly.eur,
            ip_price: IpPrice {
                monthly: raw.ip_prices.monthly.eur,
                hourly: raw.ip_prices.hourly.eur,
            },
            datacenter: raw.details.datacenter.name,
            fixed_price: raw.prices.fixed,
            next_reduce: raw.timer.reduce_next,
            next_reduce_timestamp: if raw.timer.reduce_next_timestamp == 0 {
                None
            } else {
                Some(raw.timer.reduce_next_timestamp)
            },
            traffic: raw.details.traffic,
            bandwidth: raw.details.bandwidth,
            is_ecc: raw.hardware.ram.ecc,
            is_highio: false,
            specials: raw.details.specials,
            description: raw.details.description,
            information: raw.details.information,
            dist: raw.details.os,
        })
        .collect();
    Ok(auctions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_current_auction_response() {
        let auctions = parse_auctions(
            r##"{
                "server": [{
                    "Id": 3049743,
                    "Hardware": {
                        "CPU": {"Name": "Intel Core i5-12500", "CoreCount": 1},
                        "RAM": {"RealSize": 32768, "Size": 128, "SizeUnit": "GB", "Amount": 4, "ecc": false},
                        "Storage": {
                            "RealSize": 1024,
                            "Size": 512,
                            "SizeUnit": "GB",
                            "Amount": 2,
                            "Disks": ["512 GB SSD", "512 GB SSD"],
                            "Details": {"nvme": [512, 512], "sata": [], "hdd": [], "general": [512]}
                        }
                    },
                    "Prices": {
                        "monthly": {"EUR": 57, "USD": 64},
                        "hourly": {"EUR": 0.0913, "USD": 0.1026},
                        "setup": {"EUR": 0, "USD": 0},
                        "fixed": false
                    },
                    "IPPrices": {
                        "monthly": {"EUR": 1.7, "USD": 1.9},
                        "hourly": {"EUR": 0.0027, "USD": 0.003},
                        "Amount": 1
                    },
                    "Details": {
                        "Description": ["IPv4", "Intel Core i5-12500"],
                        "Information": ["4 x RAM 32768 MB DDR4"],
                        "Specials": ["IPv4", "iNIC"],
                        "Traffic": "unlimited",
                        "Bandwidth": 1000,
                        "OS": ["Rescue system"],
                        "Datacenter": {"Name": "HEL1-DC5", "Datacenter": "#HEL1-DC5"}
                    },
                    "Timer": {"ReduceNext": 199097, "ReduceNextHr": true, "ReduceNextTimestamp": 1786058059}
                }],
                "serverCount": 1,
                "filter": {}
            }"##,
        )
        .expect("current response should parse");

        let auction = &auctions[0];
        assert_eq!(auction.id, 3049743);
        assert_eq!(auction.cpu, "Intel Core i5-12500");
        assert_eq!(auction.cpu_count, 1);
        assert_eq!(auction.ram_size, 128);
        assert_eq!(auction.hdd_size, 512);
        assert_eq!(auction.hdd_count, 2);
        assert_eq!(auction.price, 57.0);
        assert_eq!(auction.setup_price, 0.0);
        assert_eq!(auction.hourly_price, 0.0913);
        assert_eq!(auction.ip_price.monthly, 1.7);
        assert_eq!(auction.ip_price.hourly, 0.0027);
        assert_eq!(auction.datacenter, "HEL1-DC5");
        assert!(!auction.fixed_price);
        assert_eq!(auction.next_reduce, 199097);
        assert_eq!(auction.next_reduce_timestamp, Some(1786058059));
        assert_eq!(auction.traffic, "unlimited");
        assert_eq!(auction.bandwidth, 1000);
        assert!(!auction.is_ecc);
        assert!(!auction.is_highio);
        assert_eq!(auction.specials, ["IPv4", "iNIC"]);
        assert_eq!(auction.description, ["IPv4", "Intel Core i5-12500"]);
        assert_eq!(auction.information, ["4 x RAM 32768 MB DDR4"]);
        assert_eq!(auction.dist, ["Rescue system"]);
    }

    #[tokio::test]
    async fn fetch_auctions_successfully() {
        let auctions = fetch_auctions().await.expect("fetch should succeed");
        assert!(!auctions.is_empty());
        let first = &auctions[0];
        assert!(!first.cpu.is_empty());
        assert!(first.cpu_count > 0);
    }
}
