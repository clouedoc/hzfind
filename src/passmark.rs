use std::sync::LazyLock;

use serde::Deserialize;

use eyre::Result;
use regex::Regex;

use crate::hetzner_auction::HetznerAuction;

#[derive(Debug, Deserialize)]
pub struct PassmarkScore {
    pub name: String,
    pub cpumark: u32,
    pub cores: u32,
}

/// Intermediate JSON shape matching the raw API response.
#[derive(Deserialize)]
struct RawPassmarkScore {
    name: String,
    cpumark: String,
    cores: String,
}

pub static PASSMARK_SCORES: LazyLock<&'static [PassmarkScore]> = LazyLock::new(|| {
    let json = include_str!("../assets/passmark.json");
    parse_scores(json)
        .expect("failed to parse passmark.json")
        .leak()
});

fn parse_scores(json: &str) -> Result<Vec<PassmarkScore>> {
    #[derive(Deserialize)]
    struct Root {
        data: Vec<RawPassmarkScore>,
    }

    // Regexp that removes the " @ xx.xx Ghz" frequency thing from PassMark CPU names
    // which allows easier matching with Hetzner Auctions listings.
    let freq_regexp = Regex::new(" @ .+$").unwrap();
    let cpu_version_regex =
        Regex::new(r#"(?<cpu_name>Intel Xeon ..-.+) (?:v|V)(?<version>\d)"#).unwrap();
    let root: Root = serde_json::from_str(json)?;
    let scores: Vec<PassmarkScore> = root
        .data
        .into_iter()
        .map(|raw| PassmarkScore {
            cpumark: raw.cpumark.replace(',', "").parse().unwrap_or(0),
            cores: raw.cores.replace(',', "").parse().unwrap_or(0),
            name: cpu_version_regex
                .replace(
                    &freq_regexp.replace(&raw.name, ""),
                    "${cpu_name}v${version}",
                )
                .to_string(),
        })
        .collect();
    Ok(scores)
}

impl HetznerAuction {
    /// Case-insensitive exact match against PassMark CPU names.
    pub fn cpu_passmark_score(&self) -> Option<&'static PassmarkScore> {
        PASSMARK_SCORES
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&self.cpu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scores_correctly() {
        assert!(!PASSMARK_SCORES.is_empty());
        let ryzen = PASSMARK_SCORES
            .iter()
            .find(|s| s.name == "AMD Ryzen 5 3600")
            .expect("AMD Ryzen 5 3600 not found in passmark data");
        assert_eq!(ryzen.cpumark, 17_673);
        assert_eq!(ryzen.cores, 6);
    }
}
