use std::sync::LazyLock;

use serde::Deserialize;

use eyre::Result;
use regex::Regex;
use serde_json::Value;

use crate::hetzner_auction::HetznerAuction;

#[derive(Debug, Deserialize)]
pub struct PassmarkScore {
    pub name: String,
    pub cpumark: u32,
    /// Total cores: P-cores + E-cores (or just P-cores if no E-cores)
    pub cores: u32,
    /// Primary/performance cores
    pub p_cores: u32,
    /// Secondary/efficiency cores (0 for non-hybrid)
    pub e_cores: u32,
}

/// Intermediate JSON shape matching the raw API response.
#[derive(Deserialize)]
struct RawPassmarkScore {
    name: String,
    cpumark: String,
    cores: String,
    #[serde(default, rename = "secondaryCores")]
    secondary_cores: Value,
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
        .map(|raw| {
            let p_cores: u32 = raw.cores.replace(',', "").parse().unwrap_or(0);
            let e_cores: u32 = value_to_u32(&raw.secondary_cores);
            PassmarkScore {
                cpumark: raw.cpumark.replace(',', "").parse().unwrap_or(0),
                cores: p_cores + e_cores,
                p_cores,
                e_cores,
                name: cpu_version_regex
                .replace(
                    &freq_regexp.replace(&raw.name, ""),
                    "${cpu_name}v${version}",
                )
                .to_string(),
            }
        })
        .collect();
    Ok(scores)
}

/// Convert a JSON value that may be a string, integer, or null to u32.
fn value_to_u32(val: &Value) -> u32 {
    match val {
        Value::String(s) => s.replace(',', "").parse().unwrap_or(0),
        Value::Number(n) => n.as_u64().unwrap_or(0) as u32,
        _ => 0,
    }
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
        assert_eq!(ryzen.p_cores, 6);
        assert_eq!(ryzen.e_cores, 0);
    }

    #[test]
    fn parse_hybrid_cpu_scores() {
        let hx370 = PASSMARK_SCORES
            .iter()
            .find(|s| s.name == "AMD Ryzen AI 9 HX 370")
            .expect("AMD Ryzen AI 9 HX 370 not found in passmark data");
        assert_eq!(hx370.cpumark, 35_081);
        assert_eq!(hx370.p_cores, 4);
        assert_eq!(hx370.e_cores, 8);
        assert_eq!(hx370.cores, 12); // total: 4 + 8
    }
}
