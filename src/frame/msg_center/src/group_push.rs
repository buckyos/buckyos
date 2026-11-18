//! Best-effort group update delivery. The hosted change log and the member's
//! committed cursor are authoritative; queues and acknowledgements can be lost.
use crate::group_types::{invalid, missing, GroupActor, GroupState, MemberStatus, Result};
use crate::msg_center::MessageCenter;
use buckyos_api::{canonical_zone_of, same_zone, CrossZoneError, CrossZoneRequest, Principal};
use name_lib::DID;
use ndn_lib::{
    MsgObjKind, ObjId, CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON, CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Semaphore;

pub(crate) const MAX_UPDATE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_UPDATE_ENTRIES: usize = 32;
const MAX_PENDING_KEYS: usize = 256;
const MAX_READERS_PER_TARGET: usize = 1024;
const MAX_BASELINES: usize = 8192;
const MAX_CONCURRENT: usize = 4;
const DEBOUNCE: Duration = Duration::from_millis(80);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupUpdate {
    pub(crate) entries: Vec<GroupReaderUpdate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupReaderUpdate {
    pub(crate) reader_did: DID,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) member_epoch: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delta: Option<GroupMessageDelta>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupMessageDelta {
    pub(crate) previous_token: String,
    pub(crate) next_token: String,
    pub(crate) change: Value,
    pub(crate) content_type: String,
    pub(crate) object: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupUpdateAck {
    pub(crate) committed: Vec<GroupReaderAck>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupReaderAck {
    pub(crate) reader_did: DID,
    pub(crate) next_token: String,
}

#[derive(Clone, Debug)]
struct ReaderVersion {
    epoch: u32,
    position: u64,
    token: String,
}

// Each key has a single worker until its last in-flight request finishes.
// Updates arriving during that request are accumulated for the next pass.
#[derive(Debug)]
struct Queue<K, V>(Mutex<BTreeMap<K, Option<V>>>);

impl<K, V> Default for Queue<K, V> {
    fn default() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Submitted {
    Start,
    Merged,
    Dropped,
}

impl<K: Ord, V> Queue<K, V> {
    fn submit(&self, key: K, value: V, merge: impl FnOnce(&mut V, V)) -> Submitted {
        let mut pending = self.0.lock().unwrap();
        if let Some(slot) = pending.get_mut(&key) {
            if let Some(existing) = slot {
                merge(existing, value);
            } else {
                *slot = Some(value);
            }
            Submitted::Merged
        } else if pending.len() >= MAX_PENDING_KEYS {
            Submitted::Dropped
        } else {
            pending.insert(key, Some(value));
            Submitted::Start
        }
    }

    fn take(&self, key: &K) -> Option<V> {
        let mut pending = self.0.lock().unwrap();
        let next = pending.get_mut(key)?.take();
        if next.is_none() {
            pending.remove(key);
        }
        next
    }
}

#[derive(Debug)]
pub(crate) struct GroupPush {
    groups: Queue<String, u64>,
    targets: Queue<(String, String), BTreeSet<String>>,
    discovery: Semaphore,
    sends: Semaphore,
    zone_locks: crate::group_sync::KeyedLocks<String>,
    baselines: Mutex<BTreeMap<(String, String), ReaderVersion>>,
    pub(crate) peak_sends: AtomicUsize,
    pub(crate) attempts: AtomicUsize,
    failures: AtomicUsize,
    pub(crate) merged: AtomicUsize,
    pub(crate) dropped: AtomicUsize,
}

impl Default for GroupPush {
    fn default() -> Self {
        Self {
            groups: Default::default(),
            targets: Default::default(),
            discovery: Semaphore::new(MAX_CONCURRENT),
            sends: Semaphore::new(MAX_CONCURRENT),
            zone_locks: Default::default(),
            baselines: Default::default(),
            peak_sends: Default::default(),
            attempts: Default::default(),
            failures: Default::default(),
            merged: Default::default(),
            dropped: Default::default(),
        }
    }
}

impl GroupPush {
    #[cfg(test)]
    pub(crate) fn saturate_discovery(&self) {
        for index in 0..MAX_PENDING_KEYS {
            self.groups
                .submit(format!("test-pending-{index}"), 0, |_, _| {});
        }
    }

    fn count_submission(&self, outcome: &Submitted) {
        match outcome {
            Submitted::Start => {}
            Submitted::Merged => {
                self.merged.fetch_add(1, Relaxed);
            }
            Submitted::Dropped => {
                self.dropped.fetch_add(1, Relaxed);
            }
        }
    }

    pub(crate) fn observe_pull(
        &self,
        group: &DID,
        reader: &DID,
        epoch: u32,
        position: u64,
        token: &str,
    ) {
        let key = (group.to_string(), reader.to_string());
        let mut versions = self.baselines.lock().unwrap();
        if let Some(prior) = versions.get(&key) {
            if (prior.epoch, prior.position) > (epoch, position) {
                return;
            }
        } else if versions.len() == MAX_BASELINES {
            versions.pop_first();
        }
        versions.insert(
            key,
            ReaderVersion {
                epoch,
                position,
                token: token.to_owned(),
            },
        );
    }

    fn acknowledge(&self, group: &DID, entry: &GroupReaderUpdate, position: u64) {
        let Some(delta) = &entry.delta else { return };
        let mut versions = self.baselines.lock().unwrap();
        if let Some(prior) = versions.get_mut(&(group.to_string(), entry.reader_did.to_string())) {
            if Some(prior.epoch) == entry.member_epoch && prior.token == delta.previous_token {
                prior.position = position;
                prior.token = delta.next_token.clone();
            }
        }
    }
}

impl MessageCenter {
    /// Called after commit. Neither name resolution nor HTTP runs on the writer.
    pub(crate) fn schedule_group_push(&self, group: &DID, previous_seq: u64) {
        if self.cross_zone.get().is_none() {
            return;
        }
        let key = group.to_string();
        let outcome = self
            .groups
            .push
            .groups
            .submit(key.clone(), previous_seq, |old, new| *old = (*old).min(new));
        self.groups.push.count_submission(&outcome);
        if outcome != Submitted::Start {
            log::debug!("group push discovery group={key} outcome={outcome:?}");
            return;
        }
        let center = self.clone();
        let group = group.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(DEBOUNCE).await;
                let queue = &center.groups.push;
                let _permit = queue
                    .discovery
                    .acquire()
                    .await
                    .expect("discovery queue open");
                let Some(after) = queue.groups.take(&key) else {
                    break;
                };
                if let Err(error) = center.discover_group_targets(&group, after).await {
                    log::warn!("group push discovery group={key} failed: {error}");
                }
            }
        });
    }

    async fn discover_group_targets(&self, group: &DID, after: u64) -> Result<()> {
        let state = self.groups.load(group).await?.ok_or_else(missing)?;
        let mut candidates = BTreeSet::new();
        for change in state
            .changes
            .iter()
            .rev()
            .take_while(|c| c.group_seq > after)
        {
            let terminal = matches!(
                change.action(),
                Some("entity.group_deleted" | "session.deleted")
            );
            for did in &change.audience {
                let member = state.members.get(did);
                let guest = state
                    .participants
                    .get(change.session_id.as_deref().unwrap_or(""))
                    .and_then(|members| members.get(did))
                    .is_some_and(|p| p.kind == "guest" && p.state == "included");
                let removing = change
                    .subject
                    .as_ref()
                    .is_some_and(|subject| subject.to_string() == *did)
                    && matches!(
                        change.action(),
                        Some(
                            "entity.member_left"
                                | "entity.member_removed"
                                | "session.member_left"
                                | "session.member_removed"
                        )
                    );
                if terminal
                    || removing
                    || member.is_some_and(|m| m.state == MemberStatus::Active)
                    || guest
                {
                    candidates.insert(did.clone());
                }
            }
        }
        if state.group_seq > after {
            candidates.extend(
                state
                    .members
                    .values()
                    .filter(|m| {
                        m.state == MemberStatus::Active
                            && state.capability(&m.member_did, "group.read_all")
                    })
                    .map(|m| m.member_did.to_string()),
            );
        }
        let mut targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for candidate in candidates {
            let Ok(reader) = DID::from_str(&candidate) else {
                continue;
            };
            if let Some(host) = self.remote_group_reader_host(&state.host, &reader).await {
                targets.entry(host).or_default().insert(candidate);
            }
        }
        for (host, readers) in targets {
            self.schedule_group_target(group, host, readers);
        }
        Ok(())
    }

    async fn remote_group_reader_host(&self, group_host: &DID, reader: &DID) -> Option<String> {
        if reader.method == "msgtunnel" || reader == group_host {
            return None;
        }
        let resolver = self.custody.get();
        let zone = canonical_zone_of(resolver.as_ref(), reader, None)
            .await
            .ok()??;
        if same_zone(group_host, &zone) {
            return None;
        }
        self.cross_zone.get()?.zone_host(&zone).await.ok()
    }

    fn schedule_group_target(&self, group: &DID, host: String, mut readers: BTreeSet<String>) {
        let key = (group.to_string(), host.clone());
        let cap = |readers: &mut BTreeSet<String>| {
            if readers.len() > MAX_READERS_PER_TARGET {
                self.groups
                    .push
                    .dropped
                    .fetch_add(readers.len() - MAX_READERS_PER_TARGET, Relaxed);
                log::debug!(
                    "group push group={} zone={host} dropped_readers={}",
                    group.to_string(),
                    readers.len() - MAX_READERS_PER_TARGET
                );
                while readers.len() > MAX_READERS_PER_TARGET {
                    readers.pop_last();
                }
            }
        };
        cap(&mut readers);
        let outcome = self
            .groups
            .push
            .targets
            .submit(key.clone(), readers, |old, more| {
                old.extend(more);
                cap(old);
            });
        self.groups.push.count_submission(&outcome);
        if outcome != Submitted::Start {
            log::debug!(
                "group push group={} zone={host} outcome={outcome:?}",
                group.to_string()
            );
            return;
        }
        let center = self.clone();
        let group = group.clone();
        tokio::spawn(async move {
            loop {
                let queue = &center.groups.push;
                // Queue behind this Zone before taking a global permit. A
                // single offline Zone cannot occupy all send slots via many groups.
                let _zone = queue.zone_locks.lock_key(host.clone()).await;
                let _permit = queue.sends.acquire().await.expect("send queue open");
                let Some(readers) = queue.targets.take(&key) else {
                    break;
                };
                queue
                    .peak_sends
                    .fetch_max(MAX_CONCURRENT - queue.sends.available_permits(), Relaxed);
                if let Err(error) = center.send_group_updates(&group, &host, readers).await {
                    let failures = queue.failures.fetch_add(1, Relaxed) + 1;
                    log::debug!(
                        "group push group={} zone={host} failures={failures}: {error}",
                        group.to_string()
                    );
                }
                log::trace!(
                    "group push peak_concurrency={}",
                    queue.peak_sends.load(Relaxed)
                );
            }
        });
    }

    async fn prepare_reader_update(
        &self,
        state: &GroupState,
        reader: DID,
    ) -> Result<GroupReaderUpdate> {
        let member = state.members.get(&reader.to_string());
        let mut entry = GroupReaderUpdate {
            reader_did: reader.clone(),
            member_epoch: member.map(|m| m.epoch),
            delta: None,
        };
        let actor = GroupActor {
            did: reader.clone(),
            client: None,
            remote: true,
        };
        let Some(member) = member
            .filter(|m| m.state == MemberStatus::Active && state.check_client(&actor).is_ok())
        else {
            return Ok(entry);
        };
        let group = &state.group_did;
        let baseline = self
            .groups
            .push
            .baselines
            .lock()
            .unwrap()
            .get(&(group.to_string(), reader.to_string()))
            .cloned();
        let Some(prior) = baseline.filter(|prior| prior.epoch == member.epoch) else {
            return Ok(entry);
        };
        if self.groups.cursor(&prior.token, group, &reader).await? != Some(prior.position) {
            return Ok(entry);
        }
        let Ok((mut items, position)) = self
            .visible_group_changes(state, &actor, prior.position, 2, true)
            .await
        else {
            return Ok(entry);
        };
        if items.len() != 1 || items[0]["kind"] != "message" {
            return Ok(entry);
        }
        let change = items.pop().unwrap();
        let Some(id) = change["message"]["obj_id"]
            .as_str()
            .and_then(|raw| ObjId::new(raw).ok())
        else {
            return Ok(entry);
        };
        if !state
            .messages
            .get(&id.to_string())
            .is_some_and(|m| m.kind == MsgObjKind::GroupMsg && m.relation.is_none() && !m.redacted)
        {
            return Ok(entry);
        }
        // The object endpoint also checks current permissions, interval, read
        // hook, and audit obligations on a fresh copy of the hosted group.
        if self.authorize_group_message(&actor, &id).await.is_err() {
            return Ok(entry);
        }
        let Some((object_group, Some(body), jwt)) = self.groups.object(&id).await? else {
            return Ok(entry);
        };
        if object_group != *group {
            return Ok(entry);
        }
        let (content_type, object) = match jwt {
            Some(jwt) => (CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT, jwt),
            None => (CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON, body),
        };
        if object.len() <= 65_536 {
            let next_token = self
                .groups
                .save_cursor(group, &reader, position, Self::now_ms())
                .await?;
            entry.delta = Some(GroupMessageDelta {
                previous_token: prior.token,
                next_token,
                change,
                content_type: content_type.to_owned(),
                object,
            });
        }
        Ok(entry)
    }

    /// Build the next bounded batch inside the transport's request builder so
    /// login, queueing, and a transport retry all precede the reader checks.
    async fn group_update_batch(
        &self,
        group: &DID,
        host: &str,
        readers: &[String],
    ) -> Result<(usize, GroupUpdate, u64)> {
        let state = self.groups.load(group).await?.ok_or_else(missing)?;
        let mut update = GroupUpdate { entries: vec![] };
        let mut consumed = 0;
        let mut bytes = br#"{"entries":[]}"#.len();
        for raw in readers {
            let reader = DID::from_str(raw).map_err(invalid)?;
            // Custody can change while this target waits in the queue.
            if self
                .remote_group_reader_host(&state.host, &reader)
                .await
                .as_deref()
                != Some(host)
            {
                consumed += 1;
                continue;
            }
            let mut entry = self.prepare_reader_update(&state, reader).await?;
            let mut size = serde_json::to_vec(&entry).map_err(invalid)?.len() + 1;
            if bytes + size > MAX_UPDATE_BYTES && !update.entries.is_empty() {
                break;
            }
            if bytes + size > MAX_UPDATE_BYTES {
                entry.delta = None;
                size = serde_json::to_vec(&entry).map_err(invalid)?.len() + 1;
            }
            consumed += 1;
            if bytes + size <= MAX_UPDATE_BYTES {
                bytes += size;
                update.entries.push(entry);
            }
            if update.entries.len() == MAX_UPDATE_ENTRIES {
                break;
            }
        }
        // A revoke, redaction, or session edit while checking the other
        // readers invalidates this snapshot. Deliver only hints in that case.
        if update.entries.iter().any(|e| e.delta.is_some())
            && self
                .groups
                .load(group)
                .await?
                .is_none_or(|latest| latest.group_seq != state.group_seq)
        {
            for entry in &mut update.entries {
                entry.delta = None;
            }
        }
        // A one-item result from a two-item scan exhausted the frozen log.
        let position = state.changes.last().map_or(0, |c| c.group_seq);
        Ok((consumed, update, position))
    }

    async fn send_group_updates(
        &self,
        group: &DID,
        host: &str,
        readers: BTreeSet<String>,
    ) -> Result<()> {
        let client = self.cross_zone.get().ok_or_else(missing)?;
        let state = self.groups.load(group).await?.ok_or_else(missing)?;
        let principal = Principal::new(state.host);
        let path = format!("/kapi/msg-center/groups/{}/updates", group.to_string());
        let readers: Vec<_> = readers.into_iter().collect();
        let mut offset = 0;
        while offset < readers.len() {
            let sent = Mutex::new(None);
            let build = || async {
                let batch = self
                    .group_update_batch(group, host, &readers[offset..])
                    .await
                    .map_err(CrossZoneError::Target)?;
                if batch.1.entries.is_empty() {
                    return Err(CrossZoneError::Target(missing()));
                }
                let body =
                    serde_json::to_vec(&batch.1).map_err(|e| CrossZoneError::Target(invalid(e)))?;
                let push = &self.groups.push;
                let attempt = push.attempts.fetch_add(1, Relaxed) + 1;
                log::debug!("group push group={} zone={host} readers={} deltas={} attempt={attempt} merged={} dropped={} peak_concurrency={}",
                    group.to_string(), batch.1.entries.len(), batch.1.entries.iter().filter(|e| e.delta.is_some()).count(),
                    push.merged.load(Relaxed), push.dropped.load(Relaxed), push.peak_sends.load(Relaxed));
                *sent.lock().unwrap() = Some(batch);
                Ok(CrossZoneRequest::new(reqwest::Method::POST, &path)
                    .header("content-type", "application/json")
                    .body(body)
                    .timeout(DELIVERY_TIMEOUT))
            };
            // Bound login, retries and the ACK body together.
            let ack: GroupUpdateAck = tokio::time::timeout(DELIVERY_TIMEOUT, async {
                let mut response = client
                    .request_host_with(
                        host,
                        crate::cyfs_dispatch::MSG_CENTER_SERVICE,
                        &principal,
                        build,
                    )
                    .await
                    .map_err(invalid)?;
                if !response.status().is_success() {
                    return Err(invalid("group-update-refused"));
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(invalid)? {
                    if chunk.len() > MAX_UPDATE_BYTES.saturating_sub(bytes.len()) {
                        return Err(invalid("group-update-ack-too-large"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                serde_json::from_slice(&bytes).map_err(invalid)
            })
            .await
            .map_err(|_| invalid("group-update-timeout"))??;
            let (consumed, update, position) = sent
                .into_inner()
                .unwrap()
                .expect("successful request built");
            offset += consumed;
            for entry in &update.entries {
                if let Some(delta) = &entry.delta {
                    if ack.committed.iter().any(|a| {
                        a.reader_did == entry.reader_did && a.next_token == delta.next_token
                    }) {
                        self.groups.push.acknowledge(group, entry, position);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_queue_still_merges_updates_to_outstanding_keys() {
        let queue = Queue::default();
        for key in 0..MAX_PENDING_KEYS {
            assert_eq!(queue.submit(key, 1_u64, |_, _| {}), Submitted::Start);
        }
        assert_eq!(queue.take(&0), Some(1));
        assert_eq!(
            queue.submit(MAX_PENDING_KEYS, 1, |_, _| {}),
            Submitted::Dropped
        );
        assert_eq!(queue.submit(0, 3, |_, _| {}), Submitted::Merged);
        assert_eq!(
            queue.submit(0, 2, |a, b| *a = (*a).min(b)),
            Submitted::Merged
        );
        assert_eq!(queue.take(&0), Some(2));
        assert_eq!(queue.take(&0), None);
        assert_eq!(
            queue.submit(MAX_PENDING_KEYS, 1, |_, _| {}),
            Submitted::Start
        );
    }
}
