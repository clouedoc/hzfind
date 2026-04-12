use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct HetznerCloudServer {
    /// Name of the server in Hetzner Cloud (e.g. "CCX33").
    pub name: String,
    /// CPU model name (e.g. "AMD Ryzen 5 3600").
    pub cpu_name: String,
    /// RAM in GB.
    pub ram_gb: u32,
    /// HDD/SSD storage in GB.
    pub storage_gb: u32,
    /// Physical cores.
    pub cores: u32,
    /// Logical cores (threads / hardware threads, including SMT).
    pub threads: u32,
    /// PassMark CPU score.
    pub cpumark: u32,
    /// Monthly price in EUR (excl. VAT).
    pub price_monthly_eur: f64,
    /// Datacenter location (e.g. "HEL1").
    pub datacenter_location: String,
}

impl HetznerCloudServer {
    /// Price-per-euro CPU score.
    pub fn cpu_score_per_eur(&self) -> f64 {
        self.cpumark as f64 / self.price_monthly_eur
    }

    /// Price-per-euro RAM.
    pub fn ram_per_eur(&self) -> f64 {
        self.ram_gb as f64 / self.price_monthly_eur
    }

    /// Price-per-euro storage.
    pub fn storage_per_eur(&self) -> f64 {
        self.storage_gb as f64 / self.price_monthly_eur
    }
}

pub static HETZNER_CLOUD_SERVERS: LazyLock<Vec<HetznerCloudServer>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/hetzner_cloud.json"))
        .expect("failed to parse assets/hetzner_cloud.json")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_servers() {
        let servers = &*HETZNER_CLOUD_SERVERS;
        assert!(!servers.is_empty());
    }

    #[test]
    fn ccx33_fields() {
        let ccx33 = HETZNER_CLOUD_SERVERS
            .iter()
            .find(|s| s.name == "CCX33")
            .expect("CCX33 should exist");
        assert_eq!(ccx33.ram_gb, 32);
        assert_eq!(ccx33.storage_gb, 240);
        assert_eq!(ccx33.cores, 4);
        assert_eq!(ccx33.cpumark, 14698);
        assert!((ccx33.price_monthly_eur - 62.99).abs() < f64::EPSILON);
    }

    #[test]
    fn ccx33_per_eur_metrics() {
        let ccx33 = HETZNER_CLOUD_SERVERS
            .iter()
            .find(|s| s.name == "CCX33")
            .expect("CCX33 should exist");
        let expected_cpu_per_eur = 14698.0_f64 / 62.99;
        let expected_ram_per_eur = 32.0_f64 / 62.99;
        let expected_storage_per_eur = 240.0_f64 / 62.99;
        assert!((ccx33.cpu_score_per_eur() - expected_cpu_per_eur).abs() < 0.01);
        assert!((ccx33.ram_per_eur() - expected_ram_per_eur).abs() < 0.01);
        assert!((ccx33.storage_per_eur() - expected_storage_per_eur).abs() < 0.01);
    }
}
