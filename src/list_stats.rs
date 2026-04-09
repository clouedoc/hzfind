use std::collections::BTreeSet;

use serde::Serialize;

use hzfind::hetzner_auction::HetznerAuction;

#[derive(Debug, Serialize)]
pub(crate) struct ListStats {
    auctions_count: usize,
    cpu_matched_count: usize,
    cpu_unmatched_count: usize,
    cpu_unmatched_set: Vec<String>,
}

pub fn list_stats(auctions: &[HetznerAuction]) -> ListStats {
    let cpu_matched = auctions
        .iter()
        .filter(|a| a.cpu_passmark_score().is_some())
        .count();
    let cpu_unmatched_count = auctions.len() - cpu_matched;
    let unique_cpu_unmatched: Vec<String> = auctions
        .iter()
        .filter(|a| a.cpu_passmark_score().is_none())
        .map(|a| a.cpu.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    ListStats {
        auctions_count: auctions.len(),
        cpu_matched_count: cpu_matched,
        cpu_unmatched_count,
        cpu_unmatched_set: unique_cpu_unmatched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hzfind::hetzner_auction::fetch_auctions;

    #[tokio::test]
    async fn list_stats_is_valid() {
        let auctions = fetch_auctions().await.unwrap();
        let stats = list_stats(&auctions);
        assert!(stats.auctions_count > 0);
        assert!(stats.auctions_count >= stats.cpu_matched_count);
        assert_eq!(
            stats.auctions_count,
            stats.cpu_matched_count + stats.cpu_unmatched_count
        );
        assert!(stats.cpu_unmatched_set.len() <= stats.cpu_unmatched_count);
    }
}
