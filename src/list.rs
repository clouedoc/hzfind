use clap::ValueEnum;

use serde::Serialize;

use hzfind::hetzner_auction::HetznerAuction;
use hzfind::hetzner_cloud::HETZNER_CLOUD_SERVERS;
use hzfind::passmark::PassmarkScore;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SortField {
    Cpu,
    Storage,
    Ram,
}

#[derive(Debug, Default, Clone, Serialize)]
pub enum ListItemId {
    HetznerAuctions(u32),
    HetznerCloud(String),
    #[default]
    None,
}

impl std::fmt::Display for ListItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListItemId::HetznerAuctions(id) => write!(f, "SB:{id}"),
            ListItemId::HetznerCloud(name) => write!(f, "CLOUD:{name}"),
            ListItemId::None => write!(f, "—"),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ListItem {
    /// Auction ID
    pub id: ListItemId,
    /// CPU model name (e.g. "AMD Ryzen 5 3600")
    pub cpu_name: String,
    /// Number of CPUs
    pub cpu_count: u32,
    /// Total RAM in GB
    pub ram_size_gb: u32,
    /// Total storage in GB (hdd_size * hdd_count)
    pub total_storage_gb: u32,
    /// Total cores (cores_per_cpu × cpu_count). None if no PassMark match.
    pub total_cores: Option<u32>,
    /// P-cores per CPU (None if no PassMark match)
    pub p_cores: Option<u32>,
    /// E-cores per CPU (None if no PassMark match)
    pub e_cores: Option<u32>,
    /// PassMark score of a single CPU (None if no match)
    pub individual_cpu_score: Option<u32>,
    /// PassMark score × cpu_count (None if no match)
    pub total_cpu_score: Option<u32>,
    /// Total monthly price in EUR (base price + IP price). Does not include VAT.
    pub price_monthly_eur: f64,
    /// total_cpu_score / price_monthly_eur (None if no CPU score)
    pub cpu_score_per_eur: Option<f64>,
    /// total_storage_gb / price_monthly_eur
    pub storage_gb_per_eur: f64,
    /// ram_size_gb / price_monthly_eur
    pub ram_gb_per_eur: f64,
    /// Datacenter location (e.g. "HEL1-DC6")
    pub hz_datacenter_location: String,
}

pub fn build_list(auctions: &[HetznerAuction]) -> Vec<ListItem> {
    let mut items: Vec<ListItem> = auctions
        .iter()
        .map(|auction| {
            let score: Option<&PassmarkScore> = auction.cpu_passmark_score();
            let individual_cpu_score = score.map(|s| s.cpumark);
            let total_cpu_score = individual_cpu_score.map(|s| s * auction.cpu_count);
            let total_cores = score.map(|s| s.cores * auction.cpu_count);
            let p_cores = score.map(|s| s.p_cores);
            let e_cores = score.map(|s| s.e_cores);
            let price_monthly_eur = auction.price + auction.ip_price.monthly;
            let total_storage_gb = auction.hdd_size * auction.hdd_count;
            let cpu_score_per_eur = total_cpu_score.map(|s| s as f64 / price_monthly_eur);

            ListItem {
                id: ListItemId::HetznerAuctions(auction.id),
                cpu_name: auction.cpu.clone(),
                cpu_count: auction.cpu_count,
                ram_size_gb: auction.ram_size,
                total_storage_gb,
                individual_cpu_score,
                total_cpu_score,
                total_cores,
                p_cores,
                e_cores,
                price_monthly_eur,
                cpu_score_per_eur,
                storage_gb_per_eur: total_storage_gb as f64 / price_monthly_eur,
                ram_gb_per_eur: auction.ram_size as f64 / price_monthly_eur,
                hz_datacenter_location: auction.datacenter.clone(),
            }
        })
        .collect();

    // Append Hetzner Cloud servers as ListItems so they appear in listings.
    for cloud in HETZNER_CLOUD_SERVERS.iter() {
        items.push(ListItem {
            id: ListItemId::HetznerCloud(cloud.name.clone()),
            cpu_name: cloud.cpu_name.clone(),
            cpu_count: 1,
            ram_size_gb: cloud.ram_gb,
            total_storage_gb: cloud.storage_gb,
            total_cores: Some(cloud.cores),
            p_cores: Some(cloud.cores),
            e_cores: None,
            individual_cpu_score: Some(cloud.cpumark),
            total_cpu_score: Some(cloud.cpumark),
            price_monthly_eur: cloud.price_monthly_eur,
            cpu_score_per_eur: Some(cloud.cpu_score_per_eur()),
            storage_gb_per_eur: cloud.storage_per_eur(),
            ram_gb_per_eur: cloud.ram_per_eur(),
            hz_datacenter_location: cloud.datacenter_location.clone(),
        });
    }

    items
}

pub fn sort_items(items: &mut [ListItem], field: SortField) {
    match field {
        SortField::Cpu => items.sort_by(|a, b| {
            b.cpu_score_per_eur
                .unwrap_or(0.0)
                .partial_cmp(&a.cpu_score_per_eur.unwrap_or(0.0))
                .unwrap()
        }),
        SortField::Storage => {
            items.sort_by(|a, b| b.storage_gb_per_eur.total_cmp(&a.storage_gb_per_eur))
        }
        SortField::Ram => items.sort_by(|a, b| b.ram_gb_per_eur.total_cmp(&a.ram_gb_per_eur)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hzfind::hetzner_auction::fetch_auctions;
    use hzfind::passmark::PASSMARK_SCORES;

    #[tokio::test]
    async fn list_computes_fields_correctly() {
        let auctions = fetch_auctions().await.unwrap();
        let items = build_list(&auctions);
        assert!(!items.is_empty());

        let first = &items[0];
        assert_ne!(first.cpu_name, "");
        assert!(first.cpu_count > 0);
        assert!(first.ram_size_gb > 0);
        let first_auction = &auctions[0];
        assert_eq!(
            first.total_storage_gb,
            first_auction.hdd_size * first_auction.hdd_count
        );
        assert!(first.price_monthly_eur > 0.0);

        if let (Some(total), Some(indiv), Some(spe)) = (
            first.total_cpu_score,
            first.individual_cpu_score,
            first.cpu_score_per_eur,
        ) {
            assert_eq!(total, indiv * first.cpu_count);
            assert!(spe > 0.0);
        }

        if let (Some(cores), Some(indiv_score)) = (first.total_cores, first.individual_cpu_score) {
            let score = PASSMARK_SCORES
                .iter()
                .find(|s| s.cpumark == indiv_score)
                .expect("matching passmark entry not found");
            assert_eq!(cores, score.cores * first.cpu_count);
            assert_eq!(first.p_cores, Some(score.p_cores));
            assert_eq!(first.e_cores, Some(score.e_cores));
        }
    }

    #[test]
    fn sort_items_works() {
        let mut items = vec![
            {
                let mut item = ListItem::default();
                item.cpu_score_per_eur = Some(10.0);
                item.storage_gb_per_eur = 5.0;
                item.ram_gb_per_eur = 3.0;
                item
            },
            {
                let mut item = ListItem::default();
                item.cpu_score_per_eur = Some(30.0);
                item.storage_gb_per_eur = 1.0;
                item.ram_gb_per_eur = 20.0;
                item
            },
            {
                let mut item = ListItem::default();
                item.cpu_score_per_eur = Some(20.0);
                item.storage_gb_per_eur = 50.0;
                item.ram_gb_per_eur = 8.0;
                item
            },
        ];

        // Sort by CPU — descending
        sort_items(&mut items, SortField::Cpu);
        let scores: Vec<Option<f64>> = items.iter().map(|i| i.cpu_score_per_eur).collect();
        assert_eq!(scores, vec![Some(30.0), Some(20.0), Some(10.0)]);

        // Sort by Storage — descending
        sort_items(&mut items, SortField::Storage);
        let storage: Vec<f64> = items.iter().map(|i| i.storage_gb_per_eur).collect();
        assert_eq!(storage, vec![50.0, 5.0, 1.0]);

        // Sort by RAM — descending
        sort_items(&mut items, SortField::Ram);
        let ram: Vec<f64> = items.iter().map(|i| i.ram_gb_per_eur).collect();
        assert_eq!(ram, vec![20.0, 8.0, 3.0]);
    }
}
