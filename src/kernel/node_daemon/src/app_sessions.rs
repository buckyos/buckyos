//! An app process trades the LoginAssertion it is started with for a Verify
//! Hub session and refreshes that from then on. It cannot sign another one, so
//! when the refresh token ran out (the node slept or was cut off for a week)
//! only a new start, with a new assertion, lets the app back in.

use std::collections::HashSet;
use std::sync::Mutex;

use buckyos_api::{AppInstanceId, REFRESH_TOKEN_EXPIRE_SECONDS};
use log::warn;

/// An app refreshed at most one session lifetime before the node fell silent.
const SESSION_MARGIN_SECS: u64 = 3600;

pub(crate) struct AppSessions {
    last_reached: Option<u64>,
    lost: Mutex<HashSet<AppInstanceId>>,
}

impl AppSessions {
    /// `last_reached`: by an earlier node_daemon, whose apps may still run.
    pub(crate) fn new(last_reached: Option<u64>) -> Self {
        Self {
            last_reached,
            lost: Mutex::default(),
        }
    }

    /// The Zone answered this node, which runs `apps`.
    pub(crate) fn zone_reached(&mut self, now: u64, apps: impl Iterator<Item = AppInstanceId>) {
        let silent = now.saturating_sub(self.last_reached.replace(now).unwrap_or(now));
        if silent >= REFRESH_TOKEN_EXPIRE_SECONDS - SESSION_MARGIN_SECS {
            warn!("the Zone was not reached for {silent}s: app sessions ended, apps are started again");
            self.lost.lock().unwrap().extend(apps);
        }
    }

    pub(crate) fn is_lost(&self, app: &AppInstanceId) -> bool {
        self.lost.lock().unwrap().contains(app)
    }

    /// The process that held the session is gone.
    pub(crate) fn stopped(&self, app: &AppInstanceId) {
        self.lost.lock().unwrap().remove(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    fn apps() -> Vec<AppInstanceId> {
        ["notes.example.com@alice", "jarvis.example.com@alice"]
            .map(|id| id.parse().unwrap())
            .to_vec()
    }

    #[test]
    fn apps_are_started_again_only_after_their_refresh_token_ran_out() {
        let apps = apps();
        let mut sessions = AppSessions::new(None);
        let mut now = 1_800_000_000;
        for gap in [0, 5, 30 * 60, 3 * DAY, 6 * DAY] {
            now += gap;
            sessions.zone_reached(now, apps.iter().cloned());
            assert!(apps.iter().all(|app| !sessions.is_lost(app)), "{gap}");
        }
        now += 7 * DAY;
        sessions.zone_reached(now, apps.iter().cloned());
        assert!(apps.iter().all(|app| sessions.is_lost(app)));

        sessions.stopped(&apps[0]);
        sessions.zone_reached(now + 5, apps.iter().cloned());
        assert!(!sessions.is_lost(&apps[0]), "once");
        assert!(sessions.is_lost(&apps[1]), "until it was stopped");

        sessions.zone_reached(now - DAY, apps.iter().cloned());
        assert!(!sessions.is_lost(&apps[0]), "a clock set back is no gap");

        let mut restarted = AppSessions::new(Some(now - 8 * DAY));
        restarted.zone_reached(now, apps.iter().cloned());
        assert!(restarted.is_lost(&apps[0]));
    }
}
