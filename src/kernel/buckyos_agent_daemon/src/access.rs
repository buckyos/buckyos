use std::time::{Duration, Instant};

use buckyos_api::AGENT_DAEMON_MANAGE_PATH;
use url::Url;

pub struct AccessUrl {
    pub endpoint: Url,
    pub executor_id: String,
    pub secret: String,
}

pub fn valid_executor_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

impl AccessUrl {
    pub fn parse(input: &str, allow_loopback_ws: bool) -> Result<Self, String> {
        let mut endpoint = Url::parse(input.trim()).map_err(|_| "invalid access URL")?;
        let loopback = endpoint
            .host_str()
            .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
        if endpoint.scheme() != "wss"
            && !(allow_loopback_ws && endpoint.scheme() == "ws" && loopback)
        {
            return Err(
                "access URL requires WSS (loopback WS requires --allow-loopback-ws)".into(),
            );
        }
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != AGENT_DAEMON_MANAGE_PATH
        {
            return Err("invalid access URL endpoint".into());
        }
        let params: Vec<_> = endpoint.query_pairs().collect();
        if params.len() != 1 || params[0].0 != "executor_id" || !valid_executor_id(&params[0].1) {
            return Err("access URL requires one valid executor_id".into());
        }
        let executor_id = params[0].1.to_string();
        let secret = endpoint
            .fragment()
            .filter(|s| s.len() >= 32 && s.len() <= 256)
            .filter(|s| {
                s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            })
            .ok_or("invalid access credential")?
            .to_owned();
        endpoint.set_fragment(None);
        endpoint.set_query(None);
        Ok(Self {
            endpoint,
            executor_id,
            secret,
        })
    }
}

pub fn boot_ms() -> u64 {
    #[cfg(target_os = "linux")]
    let clock = libc::CLOCK_BOOTTIME;
    #[cfg(target_os = "macos")]
    let clock = libc::CLOCK_MONOTONIC_RAW;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(clock, &mut ts) } == 0 {
            return (ts.tv_sec as u64).saturating_mul(1000) + ts.tv_nsec as u64 / 1_000_000;
        }
        return u64::MAX;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    node_executor::now_ms()
}

pub struct Deadline {
    absolute: Option<u64>,
    boot: Option<u64>,
    monotonic: Option<Instant>,
    initialized: bool,
}

impl Default for Deadline {
    fn default() -> Self {
        Self {
            absolute: None,
            boot: None,
            monotonic: None,
            initialized: false,
        }
    }
}

impl Deadline {
    pub fn observe(
        &mut self,
        server_ms: u64,
        expires_at: Option<u64>,
        request_started: u64,
    ) -> Result<(), String> {
        if self.initialized && self.absolute != expires_at {
            return Err("access deadline changed; restart with a newly issued URL".into());
        }
        self.initialized = true;
        self.absolute = expires_at;
        if let Some(expiry) = expires_at {
            let remaining = expiry.saturating_sub(server_ms);
            let boot = request_started.saturating_add(remaining);
            self.boot = Some(self.boot.map_or(boot, |old| old.min(boot)));
            let mono = Instant::now() + Duration::from_millis(boot.saturating_sub(boot_ms()));
            self.monotonic = Some(self.monotonic.map_or(mono, |old| old.min(mono)));
            if self.expired() {
                return Err("access URL expired".into());
            }
        }
        Ok(())
    }

    pub fn expired(&self) -> bool {
        self.boot.is_some_and(|d| boot_ms() >= d)
            || self.monotonic.is_some_and(|d| Instant::now() >= d)
    }

    pub fn gate_until(&self, lease_ms: u64) -> u64 {
        let lease = boot_ms().saturating_add(lease_ms);
        self.boot.map_or(lease, |expiry| expiry.min(lease))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_separated_and_unsafe_urls_rejected() {
        let secret = "01234567890123456789012345678901";
        let url =
            format!("wss://zone.example{AGENT_DAEMON_MANAGE_PATH}?executor_id=exec-1#{secret}");
        let access = AccessUrl::parse(&url, false).unwrap();
        assert!(!access.endpoint.as_str().contains(secret));
        assert!(AccessUrl::parse(&url.replace("wss:", "ws:"), true).is_err());
        assert!(AccessUrl::parse(&url.replace("exec-1", ".."), false).is_err());
        assert!(
            AccessUrl::parse(&url.replace("zone.example", "secret@zone.example"), false).is_err()
        );
        assert!(AccessUrl::parse(
            &url.replace(
                "executor_id=exec-1",
                "executor_id=exec-1&executor_id=exec-2"
            ),
            false
        )
        .is_err());
    }

    #[test]
    fn reconnect_and_clock_rollback_cannot_extend_expiry() {
        let mut deadline = Deadline::default();
        let start = boot_ms();
        deadline.observe(1_000, Some(2_000), start).unwrap();
        let first = deadline.boot;
        deadline.observe(500, Some(2_000), start + 10).unwrap();
        assert_eq!(deadline.boot, first);
        assert!(deadline.observe(1_000, Some(9_000), start).is_err());
        assert!(deadline.observe(1_000, None, start).is_err());
        deadline.boot = Some(boot_ms().saturating_sub(1));
        assert!(deadline.expired());
    }

    #[test]
    fn expired_request_cannot_gain_a_lease_on_observation() {
        let mut deadline = Deadline::default();
        assert!(deadline.observe(2_000, Some(1_000), boot_ms()).is_err());
        assert!(deadline.expired());
        assert!(deadline.gate_until(30_000) <= boot_ms());
    }

    #[test]
    fn boot_clock_is_available_and_advances() {
        let before = boot_ms();
        assert_ne!(before, u64::MAX);
        std::thread::sleep(Duration::from_millis(5));
        assert!(boot_ms() >= before + 1);
    }
}
