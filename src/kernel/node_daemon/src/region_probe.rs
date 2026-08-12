use cyfs_gateway_api::{
    fetch_sn_region_probe_config, is_public_sn_probe_ip, SnRegionProbeConfig,
    SnRegionProbeConfigDocument, SnRegionProbeConfigFetch,
};
use futures::{stream, StreamExt};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::{lookup_host, TcpStream};
use tokio::sync::RwLock;
use tokio::time::{timeout, timeout_at, Instant as TokioInstant};
use url::Url;

const MAX_CONNECT_TIMEOUT: Duration = Duration::from_millis(2_000);
const MAX_ROUND_TIMEOUT: Duration = Duration::from_millis(5_000);
const MAX_CONCURRENCY: usize = 16;
const MAX_TOTAL_URLS: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionProbePhase {
    Idle,
    Running,
    Completed,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionSelectionSource {
    Probe,
    Cache,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionConfidence {
    High,
    Low,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableRegion {
    pub region_id: String,
    pub priority: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegionMeasurement {
    pub region_id: String,
    pub priority: i32,
    pub score_ms: Option<u64>,
    pub valid_urls: usize,
    pub total_urls: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegionProbeStatus {
    pub phase: RegionProbePhase,
    pub region: Option<String>,
    pub source: RegionSelectionSource,
    pub config_version: Option<String>,
    pub confidence: RegionConfidence,
    pub measured_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub available_regions: Vec<AvailableRegion>,
    pub regions: Vec<RegionMeasurement>,
}

impl RegionProbeStatus {
    fn idle() -> Self {
        Self {
            phase: RegionProbePhase::Idle,
            region: None,
            source: RegionSelectionSource::None,
            config_version: None,
            confidence: RegionConfidence::None,
            measured_at: None,
            expires_at: None,
            available_regions: Vec::new(),
            regions: Vec::new(),
        }
    }

    fn unavailable(
        config_version: Option<String>,
        available_regions: Vec<AvailableRegion>,
    ) -> Self {
        Self {
            phase: RegionProbePhase::Unavailable,
            config_version,
            available_regions,
            ..Self::idle()
        }
    }
}

#[derive(Clone)]
struct CachedConfig {
    sn_url: String,
    document: SnRegionProbeConfigDocument,
}

#[derive(Clone)]
struct CachedResult {
    sn_url: String,
    config_version: String,
    network_fingerprint: String,
    expires_at: u64,
    status: RegionProbeStatus,
}

struct RegionProbeInner {
    generation: u64,
    status: RegionProbeStatus,
    cached_config: Option<CachedConfig>,
    cached_result: Option<CachedResult>,
}

#[derive(Clone)]
pub struct RegionProbeController {
    sn_url: String,
    inner: Arc<RwLock<RegionProbeInner>>,
}

impl RegionProbeController {
    pub fn new(sn_url: String) -> Self {
        Self {
            sn_url,
            inner: Arc::new(RwLock::new(RegionProbeInner {
                generation: 0,
                status: RegionProbeStatus::idle(),
                cached_config: None,
                cached_result: None,
            })),
        }
    }

    pub async fn status(&self) -> RegionProbeStatus {
        self.inner.read().await.status.clone()
    }

    pub async fn start(&self, force: bool) -> RegionProbeStatus {
        let network_fingerprint = network_fingerprint().await;
        let (generation, status) = {
            let mut inner = self.inner.write().await;
            if inner.status.phase == RegionProbePhase::Running {
                return inner.status.clone();
            }
            inner.generation = inner.generation.wrapping_add(1);
            inner.status = RegionProbeStatus {
                phase: RegionProbePhase::Running,
                ..RegionProbeStatus::idle()
            };
            (inner.generation, inner.status.clone())
        };

        let controller = self.clone();
        tokio::spawn(async move {
            controller.run(generation, network_fingerprint, force).await;
        });
        status
    }

    async fn run(&self, generation: u64, network_fingerprint: Option<String>, force: bool) {
        let config = match self.load_config().await {
            Ok(Some(config)) => config,
            Ok(None) => {
                info!("SN does not publish a Region probe config");
                self.finish(
                    generation,
                    RegionProbeStatus::unavailable(None, Vec::new()),
                    None,
                )
                .await;
                return;
            }
            Err(error) => {
                warn!("Region probe config is unavailable: {}", error);
                self.finish(
                    generation,
                    RegionProbeStatus::unavailable(None, Vec::new()),
                    None,
                )
                .await;
                return;
            }
        };

        let available_regions = available_regions(&config);
        self.update_running_config(
            generation,
            config.config_version.clone(),
            available_regions.clone(),
        )
        .await;

        if !force {
            if let (Some(fingerprint), Some(cached)) =
                (network_fingerprint.as_deref(), self.cached_result().await)
            {
                if cached.sn_url == self.sn_url
                    && cached.config_version == config.config_version
                    && cached.network_fingerprint == fingerprint
                    && cached.expires_at > unix_timestamp()
                    && cached.status.region.as_ref().is_some_and(|region| {
                        config.regions.iter().any(|item| &item.region_id == region)
                    })
                {
                    let mut status = cached.status;
                    status.source = RegionSelectionSource::Cache;
                    self.finish(generation, status, None).await;
                    return;
                }
            }
        }

        let status = run_probe_round(&config).await;
        let cache = match (
            network_fingerprint,
            status.region.as_ref(),
            status.expires_at,
        ) {
            (Some(fingerprint), Some(_), Some(expires_at)) => Some(CachedResult {
                sn_url: self.sn_url.clone(),
                config_version: config.config_version.clone(),
                network_fingerprint: fingerprint,
                expires_at,
                status: status.clone(),
            }),
            _ => None,
        };
        self.finish(generation, status, cache).await;
    }

    async fn load_config(&self) -> Result<Option<SnRegionProbeConfig>, String> {
        let cached = self.inner.read().await.cached_config.clone();
        let etag = cached
            .as_ref()
            .filter(|item| item.sn_url == self.sn_url)
            .and_then(|item| item.document.etag.as_deref());

        match fetch_sn_region_probe_config(self.sn_url.as_str(), etag).await {
            Ok(SnRegionProbeConfigFetch::Modified(document)) => {
                let config = document.config.clone();
                info!(
                    "Loaded Region probe config {} from target SN",
                    config.config_version
                );
                self.inner.write().await.cached_config = Some(CachedConfig {
                    sn_url: self.sn_url.clone(),
                    document,
                });
                Ok(Some(config))
            }
            Ok(SnRegionProbeConfigFetch::NotModified) => cached
                .filter(|item| item.sn_url == self.sn_url)
                .ok_or_else(|| "SN returned 304 without a matching cached config".to_string())
                .and_then(|item| {
                    item.document.config.validate()?;
                    info!(
                        "Using ETag-matched cached Region probe config {}",
                        item.document.config.config_version
                    );
                    Ok(Some(item.document.config))
                }),
            Ok(SnRegionProbeConfigFetch::NotConfigured) => {
                let mut inner = self.inner.write().await;
                inner.cached_config = None;
                inner.cached_result = None;
                Ok(None)
            }
            Err(error) => {
                if let Some(item) = cached.filter(|item| item.sn_url == self.sn_url) {
                    if item.document.config.validate().is_ok() {
                        warn!(
                            "Using cached Region probe config after fetch failure: {}",
                            error
                        );
                        return Ok(Some(item.document.config));
                    }
                }
                Err(error.to_string())
            }
        }
    }

    async fn cached_result(&self) -> Option<CachedResult> {
        self.inner.read().await.cached_result.clone()
    }

    async fn update_running_config(
        &self,
        generation: u64,
        config_version: String,
        available_regions: Vec<AvailableRegion>,
    ) {
        let mut inner = self.inner.write().await;
        if inner.generation == generation && inner.status.phase == RegionProbePhase::Running {
            inner.status.config_version = Some(config_version);
            inner.status.available_regions = available_regions;
        }
    }

    async fn finish(
        &self,
        generation: u64,
        status: RegionProbeStatus,
        cached_result: Option<CachedResult>,
    ) {
        let mut inner = self.inner.write().await;
        if inner.generation != generation {
            return;
        }
        if let Some(cached_result) = cached_result {
            inner.cached_result = Some(cached_result);
        }
        inner.status = status;
    }
}

#[derive(Clone)]
struct ProbeTarget {
    region_index: usize,
    url_index: usize,
    region_id: String,
    probe_id: String,
    url: String,
}

#[derive(Clone)]
struct ResolvedTarget {
    target: ProbeTarget,
    address: SocketAddr,
}

#[derive(Clone)]
struct SampleTarget {
    target: ResolvedTarget,
    sample_index: usize,
}

fn build_probe_targets(config: &SnRegionProbeConfig) -> Vec<ProbeTarget> {
    let max_urls = config
        .regions
        .iter()
        .map(|region| region.probe_urls.len())
        .max()
        .unwrap_or_default();
    let mut targets = Vec::new();
    for url_index in 0..max_urls {
        for (region_index, region) in config.regions.iter().enumerate() {
            let Some(probe) = region.probe_urls.get(url_index) else {
                continue;
            };
            targets.push(ProbeTarget {
                region_index,
                url_index,
                region_id: region.region_id.clone(),
                probe_id: probe.id.clone(),
                url: probe.url.clone(),
            });
            if targets.len() == MAX_TOTAL_URLS {
                return targets;
            }
        }
    }
    targets
}

async fn resolve_target(target: ProbeTarget) -> Option<ResolvedTarget> {
    let parsed = match Url::parse(target.url.as_str()) {
        Ok(parsed) => parsed,
        Err(error) => {
            debug!(
                "Region probe {} URL parse failed: {}",
                target.probe_id, error
            );
            return None;
        }
    };
    let Some(host) = parsed.host_str() else {
        return None;
    };
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addresses = match lookup_host((host, port)).await {
        Ok(addresses) => addresses,
        Err(error) => {
            debug!(
                "Region probe {} DNS resolution failed: {}",
                target.probe_id, error
            );
            return None;
        }
    };
    let address = select_public_ipv4(addresses);
    if address.is_none() {
        debug!(
            "Region probe {} DNS resolution returned no safe public IPv4 address",
            target.probe_id
        );
    } else {
        debug!(
            "Region probe {} resolved to a safe public IPv4 address",
            target.probe_id
        );
    }
    address.map(|address| ResolvedTarget { target, address })
}

fn select_public_ipv4(addresses: impl IntoIterator<Item = SocketAddr>) -> Option<SocketAddr> {
    addresses.into_iter().find(|address| {
        matches!(address.ip(), IpAddr::V4(_)) && is_public_sn_probe_ip(address.ip())
    })
}

async fn measure_sample(
    sample: SampleTarget,
    connect_timeout: Duration,
) -> (SampleTarget, Option<u64>) {
    let started = Instant::now();
    let result = timeout(connect_timeout, TcpStream::connect(sample.target.address)).await;
    let elapsed_ms = started.elapsed().as_millis().max(1) as u64;
    let value = match result {
        Ok(Ok(stream)) => {
            drop(stream);
            debug!(
                "Region probe {} sample {} connected in {} ms",
                sample.target.target.probe_id,
                sample.sample_index + 1,
                elapsed_ms
            );
            Some(elapsed_ms)
        }
        Ok(Err(error)) => {
            debug!(
                "Region probe {} sample {} connect failed: {}",
                sample.target.target.probe_id,
                sample.sample_index + 1,
                error
            );
            None
        }
        Err(_) => {
            debug!(
                "Region probe {} sample {} timed out",
                sample.target.target.probe_id,
                sample.sample_index + 1
            );
            None
        }
    };
    (sample, value)
}

async fn run_probe_round(config: &SnRegionProbeConfig) -> RegionProbeStatus {
    let max_concurrency = config.policy.max_concurrency.min(MAX_CONCURRENCY).max(1);
    let round_timeout =
        Duration::from_millis(config.policy.round_timeout_ms).min(MAX_ROUND_TIMEOUT);
    let connect_timeout =
        Duration::from_millis(config.policy.connect_timeout_ms).min(MAX_CONNECT_TIMEOUT);
    let deadline = TokioInstant::now() + round_timeout;
    let targets = build_probe_targets(config);
    let target_counts = target_counts(&targets);

    let mut resolutions = stream::iter(targets)
        .map(resolve_target)
        .buffer_unordered(max_concurrency);
    let mut resolved = Vec::new();
    while let Ok(Some(result)) = timeout_at(deadline, resolutions.next()).await {
        if let Some(result) = result {
            resolved.push(result);
        }
    }
    drop(resolutions);

    let samples_per_url = usize::from(config.policy.samples_per_url.min(3));
    let mut samples = Vec::with_capacity(resolved.len() * samples_per_url);
    for sample_index in 0..samples_per_url {
        for target in &resolved {
            samples.push(SampleTarget {
                target: target.clone(),
                sample_index,
            });
        }
    }
    samples.sort_by_key(|sample| {
        (
            sample.sample_index,
            sample.target.target.url_index,
            sample.target.target.region_index,
        )
    });

    let mut measurements = stream::iter(samples)
        .map(|sample| measure_sample(sample, connect_timeout))
        .buffer_unordered(max_concurrency);
    let mut samples_by_url: HashMap<(usize, usize), Vec<u64>> = HashMap::new();
    while let Ok(Some((sample, value))) = timeout_at(deadline, measurements.next()).await {
        if let Some(value) = value {
            samples_by_url
                .entry((
                    sample.target.target.region_index,
                    sample.target.target.url_index,
                ))
                .or_default()
                .push(value);
        }
    }

    let regions = score_regions(config, &target_counts, &samples_by_url);
    let (region, confidence) = select_region(
        &regions,
        config.policy.minimum_valid_urls,
        config.policy.confident_ratio,
    );
    let measured_at = unix_timestamp();
    let config_expires_at = config.expires_at.timestamp().max(0) as u64;
    let expires_at = measured_at
        .saturating_add(config.policy.cache_ttl_sec.min(21_600))
        .min(config_expires_at);
    for measurement in &regions {
        info!(
            "Region probe score: region={}, score_ms={:?}, valid_urls={}/{}",
            measurement.region_id,
            measurement.score_ms,
            measurement.valid_urls,
            measurement.total_urls
        );
    }
    info!(
        "Region probe selected {:?} with {:?} confidence",
        region, confidence
    );

    RegionProbeStatus {
        phase: RegionProbePhase::Completed,
        region,
        source: if confidence == RegionConfidence::None {
            RegionSelectionSource::None
        } else {
            RegionSelectionSource::Probe
        },
        config_version: Some(config.config_version.clone()),
        confidence,
        measured_at: Some(measured_at),
        expires_at: Some(expires_at),
        available_regions: available_regions(config),
        regions,
    }
}

fn target_counts(targets: &[ProbeTarget]) -> HashMap<usize, usize> {
    let mut counts = HashMap::new();
    for target in targets {
        *counts.entry(target.region_index).or_default() += 1;
    }
    counts
}

fn score_regions(
    config: &SnRegionProbeConfig,
    target_counts: &HashMap<usize, usize>,
    samples_by_url: &HashMap<(usize, usize), Vec<u64>>,
) -> Vec<RegionMeasurement> {
    config
        .regions
        .iter()
        .enumerate()
        .map(|(region_index, region)| {
            let mut url_scores = samples_by_url
                .iter()
                .filter_map(|(&(sample_region_index, _), samples)| {
                    (sample_region_index == region_index)
                        .then(|| samples.iter().copied().min())
                        .flatten()
                })
                .collect::<Vec<_>>();
            url_scores.sort_unstable();
            RegionMeasurement {
                region_id: region.region_id.clone(),
                priority: region.priority,
                score_ms: match url_scores.len() {
                    0 => None,
                    1 => url_scores.first().copied(),
                    _ => url_scores.get(1).copied(),
                },
                valid_urls: url_scores.len(),
                total_urls: target_counts
                    .get(&region_index)
                    .copied()
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn select_region(
    measurements: &[RegionMeasurement],
    minimum_valid_urls: usize,
    confident_ratio: f64,
) -> (Option<String>, RegionConfidence) {
    let mut ranked = measurements
        .iter()
        .filter(|measurement| measurement.score_ms.is_some())
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        left.score_ms
            .cmp(&right.score_ms)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.region_id.cmp(&right.region_id))
    });
    let Some(best) = ranked.first() else {
        return (None, RegionConfidence::None);
    };
    let high_confidence = ranked.get(1).is_some_and(|second| {
        best.valid_urls >= minimum_valid_urls
            && (best.score_ms.unwrap_or(u64::MAX) as f64
                / second.score_ms.unwrap_or(1).max(1) as f64)
                <= confident_ratio
    });
    (
        Some(best.region_id.clone()),
        if high_confidence {
            RegionConfidence::High
        } else {
            RegionConfidence::Low
        },
    )
}

fn available_regions(config: &SnRegionProbeConfig) -> Vec<AvailableRegion> {
    config
        .regions
        .iter()
        .map(|region| AvailableRegion {
            region_id: region.region_id.clone(),
            priority: region.priority,
        })
        .collect()
}

async fn network_fingerprint() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        let mut interfaces = if_addrs::get_if_addrs()
            .ok()?
            .into_iter()
            .filter(|interface| !interface.is_loopback())
            .map(|interface| format!("{}|{}", interface.name, interface.ip()))
            .collect::<Vec<_>>();
        interfaces.sort();
        interfaces.dedup();
        if interfaces.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        for interface in interfaces {
            hasher.update(interface.as_bytes());
            hasher.update([0]);
        }
        Some(hex::encode(hasher.finalize()))
    })
    .await
    .ok()
    .flatten()
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, Utc};
    use cyfs_gateway_api::{
        SnRegionProbeIpFamily, SnRegionProbeMethod, SnRegionProbePolicy, SnRegionProbeRegion,
        SnRegionProbeUrl,
    };
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn config() -> SnRegionProbeConfig {
        let now = Utc::now();
        SnRegionProbeConfig {
            schema_version: 1,
            config_version: "test-v1".to_string(),
            generated_at: now - ChronoDuration::minutes(1),
            expires_at: now + ChronoDuration::hours(1),
            policy: SnRegionProbePolicy {
                probe_method: SnRegionProbeMethod::TcpConnect,
                samples_per_url: 2,
                connect_timeout_ms: 1_500,
                round_timeout_ms: 3_000,
                max_concurrency: 8,
                ip_family: SnRegionProbeIpFamily::Ipv4,
                minimum_valid_urls: 2,
                confident_ratio: 0.75,
                cache_ttl_sec: 21_600,
            },
            regions: ["jp", "us-west"]
                .into_iter()
                .enumerate()
                .map(|(region_index, region_id)| SnRegionProbeRegion {
                    region_id: region_id.to_string(),
                    priority: 100 - region_index as i32,
                    probe_urls: (0..3)
                        .map(|url_index| SnRegionProbeUrl {
                            id: format!("{}-{}", region_id, url_index),
                            url: format!("https://{}.{}.example/", url_index, region_id),
                            provider: None,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn targets_are_interleaved_by_region() {
        let targets = build_probe_targets(&config());
        let order = targets
            .iter()
            .map(|target| (target.url_index, target.region_id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                (0, "jp"),
                (0, "us-west"),
                (1, "jp"),
                (1, "us-west"),
                (2, "jp"),
                (2, "us-west")
            ]
        );
    }

    #[test]
    fn unsafe_or_non_ipv4_addresses_are_rejected() {
        let addresses = vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 443),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 443),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443),
        ];
        assert_eq!(
            select_public_ipv4(addresses),
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443))
        );
    }

    #[test]
    fn region_uses_second_fastest_url_and_stable_ranking() {
        let config = config();
        let targets = build_probe_targets(&config);
        let counts = target_counts(&targets);
        let samples = HashMap::from([
            ((0, 0), vec![8, 10]),
            ((0, 1), vec![18, 20]),
            ((0, 2), vec![28, 30]),
            ((1, 0), vec![25, 27]),
            ((1, 1), vec![35, 37]),
            ((1, 2), vec![45, 47]),
        ]);
        let measurements = score_regions(&config, &counts, &samples);
        assert_eq!(measurements[0].score_ms, Some(18));
        assert_eq!(measurements[1].score_ms, Some(35));
        assert_eq!(
            select_region(&measurements, 2, 0.75),
            (Some("jp".to_string()), RegionConfidence::High)
        );
    }

    #[test]
    fn one_valid_url_is_usable_but_low_confidence() {
        let measurements = vec![RegionMeasurement {
            region_id: "jp".to_string(),
            priority: 100,
            score_ms: Some(20),
            valid_urls: 1,
            total_urls: 2,
        }];
        assert_eq!(
            select_region(&measurements, 2, 0.75),
            (Some("jp".to_string()), RegionConfidence::Low)
        );
    }

    #[test]
    fn all_failed_regions_produce_no_selection() {
        let measurements = vec![RegionMeasurement {
            region_id: "jp".to_string(),
            priority: 100,
            score_ms: None,
            valid_urls: 0,
            total_urls: 2,
        }];
        assert_eq!(
            select_region(&measurements, 2, 0.75),
            (None, RegionConfidence::None)
        );
    }
}
