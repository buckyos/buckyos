use crate::msg_box_db::MsgBoxDbMgr;
use buckyos_api::{
    AccessDecision, AccessGroupLevel, AccountBinding, Contact, ContactPatch, ContactQuery,
    ContactSource, GrantTemporaryAccessResult, ImportContactEntry, ImportReport, RdbBackend,
    SetGroupSubscribersResult, TemporaryGrant, TemporaryGrantOutcome,
};
use kRPC::RPCErrors;
use log::info;
use name_lib::DID;
use serde_json::Value;
use sqlx::{AnyPool, Row};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

const SYSTEM_OWNER_SCOPE: &str = "__system__";
const METADATA_DID_SEQ_KEY: &str = "did_seq";
/// `contact_metadata` key prefix for the per-owner alias (tombstone) index.
const METADATA_ALIAS_INDEX_PREFIX: &str = "alias_index:";

const DEFAULT_CONTACT_LIST_LIMIT: usize = 100;
const MAX_CONTACT_LIST_LIMIT: usize = 1000;
const DEFAULT_GROUP_SUBSCRIBER_LIMIT: usize = 100;
const MAX_GROUP_SUBSCRIBER_LIMIT: usize = 1000;

#[derive(Debug, Default, Clone)]
struct ContactStore {
    contacts: HashMap<DID, Contact>,
    binding_index: HashMap<String, DID>,
    group_subscribers: HashMap<DID, Vec<DID>>,
    /// Tombstone map: merged-away source DID -> current canonical DID. Lets old
    /// `MsgObject.from/to` and old session peer DIDs still resolve after a merge
    /// without rewriting history. See doc/message_hub/Contact Mgr.md (merge).
    alias_index: HashMap<DID, DID>,
}

#[derive(Debug, Clone)]
struct MessageTunnelEndpointDid {
    did: DID,
    account_type: String,
    tunnel_instance_id: String,
    encoded_account_id: String,
}

#[derive(Debug, Clone)]
pub struct ZoneUserContactSeed {
    pub did: DID,
    pub name: String,
    pub note: Option<String>,
    pub bindings: Vec<AccountBinding>,
    pub groups: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ContactMgr {
    stores: Arc<RwLock<HashMap<String, ContactStore>>>,
    did_seq: Arc<AtomicU64>,
    msg_box_db: MsgBoxDbMgr,
}

impl ContactMgr {
    /// Build a ContactMgr that shares the msg-center rdb pool owned by
    /// `msg_box_db`. The schema was already applied when the pool was opened,
    /// so this only seeds the in-memory did-seq counter from the
    /// `contact_metadata` table.
    pub async fn new_with_msg_box(msg_box_db: MsgBoxDbMgr) -> std::result::Result<Self, RPCErrors> {
        let mgr = Self {
            stores: Arc::new(RwLock::new(HashMap::new())),
            did_seq: Arc::new(AtomicU64::new(1)),
            msg_box_db,
        };

        let next_seq = mgr.load_next_did_seq().await?;
        mgr.did_seq.store(next_seq, Ordering::SeqCst);

        Ok(mgr)
    }

    fn pool(&self) -> &AnyPool {
        self.msg_box_db.pool()
    }

    fn backend(&self) -> RdbBackend {
        self.msg_box_db.backend()
    }

    /// Translate `?` placeholders into `$N` form for postgres.
    fn render_sql(&self, sql: &str) -> String {
        match self.backend() {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    pub async fn resolve_did(
        &self,
        platform: String,
        account_id: String,
        profile_hint: Option<Value>,
        owner: Option<DID>,
    ) -> std::result::Result<DID, RPCErrors> {
        let platform = platform.trim().to_string();
        let account_id = account_id.trim().to_string();
        if platform.is_empty() || account_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "platform and account_id are required".to_string(),
            ));
        }

        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let endpoint_did = Self::message_tunnel_endpoint_did(&account_id, profile_hint.as_ref())?;

        if let Some(endpoint_did) = endpoint_did {
            let did = endpoint_did.did.clone();
            let snapshot = {
                let mut stores = self.stores.write().await;
                let store = stores.get_mut(&owner_key).ok_or_else(|| {
                    RPCErrors::ReasonError("contact store missing after load".to_string())
                })?;
                let now_ms = Self::now_ms();

                if let Some(contact) = store.contacts.get_mut(&did) {
                    if let Some(binding) = Self::find_binding_mut(contact, &platform, &account_id) {
                        binding.last_active_at = now_ms;
                        if let Some(display_id) = Self::extract_hint_string(
                            profile_hint.as_ref(),
                            &["display_id", "username"],
                        ) {
                            binding.display_id = display_id;
                        }
                        if let Some(tunnel_instance_id) = Self::extract_hint_string(
                            profile_hint.as_ref(),
                            &["tunnel_instance_id", "tunnel"],
                        ) {
                            binding.tunnel_instance_id = tunnel_instance_id;
                        }
                        Self::merge_hint_meta(binding, profile_hint.as_ref());
                        Self::merge_endpoint_meta(binding, &endpoint_did, &did);
                    } else {
                        contact.bindings.push(Self::build_endpoint_binding(
                            platform.clone(),
                            account_id.clone(),
                            profile_hint.as_ref(),
                            now_ms,
                            &endpoint_did,
                            &did,
                        ));
                    }
                    contact.updated_at = now_ms;
                } else {
                    let mut contact = Self::build_shadow_contact(
                        did.clone(),
                        platform.clone(),
                        account_id.clone(),
                        profile_hint.as_ref(),
                        now_ms,
                    );
                    if let Some(binding) = contact.bindings.first_mut() {
                        Self::merge_endpoint_meta(binding, &endpoint_did, &did);
                    }
                    store.contacts.insert(did.clone(), contact);
                }

                store.binding_index = Self::rebuild_binding_index(&store.contacts);
                store.clone()
            };

            self.save_owner_store(&owner_key, &snapshot).await?;
            return Ok(did);
        }

        // Some paths allocate a new DID, which requires a DB round-trip that
        // cannot happen while the cache write guard is held.
        let (did, needs_generation, platform_for_gen, account_for_gen) = {
            let stores = self.stores.read().await;
            let store = stores.get(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let binding_key = Self::binding_key(&platform, &account_id);
            if let Some(existing_did) = store.binding_index.get(&binding_key) {
                (
                    Some(existing_did.clone()),
                    false,
                    platform.clone(),
                    account_id.clone(),
                )
            } else {
                (None, true, platform.clone(), account_id.clone())
            }
        };

        let did = if needs_generation {
            self.generate_contact_did(owner.as_ref(), &platform_for_gen, &account_for_gen)
                .await?
        } else {
            did.unwrap()
        };

        let snapshot = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let now_ms = Self::now_ms();
            let binding_key = Self::binding_key(&platform, &account_id);

            if !needs_generation {
                if let Some(contact) = store.contacts.get_mut(&did) {
                    if let Some(binding) = Self::find_binding_mut(contact, &platform, &account_id) {
                        binding.last_active_at = now_ms;
                        if let Some(display_id) = Self::extract_hint_string(
                            profile_hint.as_ref(),
                            &["display_id", "username"],
                        ) {
                            binding.display_id = display_id;
                        }
                        if let Some(tunnel_instance_id) = Self::extract_hint_string(
                            profile_hint.as_ref(),
                            &["tunnel_instance_id", "tunnel"],
                        ) {
                            binding.tunnel_instance_id = tunnel_instance_id;
                        }
                        Self::merge_hint_meta(binding, profile_hint.as_ref());
                    }
                    contact.updated_at = now_ms;
                    store.binding_index = Self::rebuild_binding_index(&store.contacts);
                    store.clone()
                } else {
                    // Stale binding index pointed at a missing contact — fall through and create.
                    store.binding_index.remove(&binding_key);
                    let contact = Self::build_shadow_contact(
                        did.clone(),
                        platform.clone(),
                        account_id.clone(),
                        profile_hint.as_ref(),
                        now_ms,
                    );
                    store.contacts.insert(did.clone(), contact);
                    store.binding_index = Self::rebuild_binding_index(&store.contacts);
                    store.clone()
                }
            } else {
                let contact = Self::build_shadow_contact(
                    did.clone(),
                    platform.clone(),
                    account_id.clone(),
                    profile_hint.as_ref(),
                    now_ms,
                );
                store.contacts.insert(did.clone(), contact);
                store.binding_index.insert(binding_key, did.clone());
                store.clone()
            }
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(did)
    }

    pub async fn get_preferred_binding(
        &self,
        did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<AccountBinding, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;
        let contact = store.contacts.get(&did).ok_or_else(|| {
            RPCErrors::ReasonError(format!("contact not found for did {}", did.to_string()))
        })?;

        contact
            .bindings
            .iter()
            .max_by_key(|binding| binding.last_active_at)
            .cloned()
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "preferred binding not found for did {}",
                    did.to_string()
                ))
            })
    }

    /// Deterministically construct the second-level endpoint DID for a platform
    /// account. Does not require a pre-existing binding. Same inputs always
    /// produce the same `did:msgtunnel:*`.
    pub async fn resolve_endpoint_did(
        &self,
        platform: String,
        account_id: String,
        account_type: String,
        tunnel_instance_id: String,
        _owner: Option<DID>,
    ) -> std::result::Result<DID, RPCErrors> {
        let account_id = account_id.trim();
        if platform.trim().is_empty() || account_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "platform and account_id are required".to_string(),
            ));
        }
        let hint = serde_json::json!({
            "account_type": account_type,
            "tunnel_instance_id": tunnel_instance_id,
        });
        let endpoint =
            Self::message_tunnel_endpoint_did(account_id, Some(&hint))?.ok_or_else(|| {
                RPCErrors::ParseRequestError(
                    "account_type and tunnel_instance_id are required to build endpoint did"
                        .to_string(),
                )
            })?;
        Ok(endpoint.did)
    }

    /// Construction-time target resolver. `selector` matches a binding's
    /// `tunnel_instance_id` first, then `platform`. Must match exactly one endpoint
    /// binding — never falls back.
    pub async fn resolve_target(
        &self,
        contact_did: DID,
        selector: String,
        owner: Option<DID>,
    ) -> std::result::Result<DID, RPCErrors> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "selector is required".to_string(),
            ));
        }
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;
        let contact = store.contacts.get(&contact_did).ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "contact not found for did {}",
                contact_did.to_string()
            ))
        })?;

        let mut matches: Vec<&AccountBinding> = contact
            .bindings
            .iter()
            .filter(|binding| {
                binding.endpoint_did.is_some()
                    && (binding.tunnel_instance_id.eq_ignore_ascii_case(selector)
                        || binding.platform.eq_ignore_ascii_case(selector))
            })
            .collect();
        // Prefer an exact tunnel_instance_id match when both kinds are present.
        if matches
            .iter()
            .any(|b| b.tunnel_instance_id.eq_ignore_ascii_case(selector))
        {
            matches.retain(|b| b.tunnel_instance_id.eq_ignore_ascii_case(selector));
        }

        match matches.as_slice() {
            [] => Err(RPCErrors::ReasonError(format!(
                "no endpoint binding for did {} matches selector '{}'",
                contact_did.to_string(),
                selector
            ))),
            [single] => single.endpoint_did.clone().ok_or_else(|| {
                RPCErrors::ReasonError("matched binding has no endpoint did".to_string())
            }),
            _ => Err(RPCErrors::ReasonError(format!(
                "selector '{}' matches multiple endpoint bindings for did {}; refine it",
                selector,
                contact_did.to_string()
            ))),
        }
    }

    /// Reverse lookup: which canonical/contact DID owns this endpoint DID?
    pub async fn resolve_contact_for_endpoint(
        &self,
        endpoint_did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<Option<DID>, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;

        // A shadow contact's own DID can be the endpoint DID; otherwise scan
        // bindings for a matching endpoint_did.
        if store.contacts.contains_key(&endpoint_did) {
            return Ok(Some(endpoint_did));
        }
        for contact in store.contacts.values() {
            if contact
                .bindings
                .iter()
                .any(|binding| binding.endpoint_did.as_ref() == Some(&endpoint_did))
            {
                return Ok(Some(contact.did.clone()));
            }
        }
        Ok(None)
    }

    /// Resolve a possibly merged-away (alias/tombstone) DID to its current
    /// canonical DID. Returns the input unchanged when it is not an alias.
    pub async fn resolve_canonical_did(
        &self,
        did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<DID, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;
        Ok(Self::canonical_did_in_store(store, &did))
    }

    /// List all alias DIDs (merged-away sources) that now resolve to `canonical_did`.
    pub async fn list_alias_dids(
        &self,
        canonical_did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<Vec<DID>, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;
        let mut aliases: Vec<DID> = store
            .alias_index
            .iter()
            .filter(|(_, canonical)| *canonical == &canonical_did)
            .map(|(alias, _)| alias.clone())
            .collect();
        aliases.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
        Ok(aliases)
    }

    /// Follow the alias chain to the terminal canonical DID (bounded).
    fn canonical_did_in_store(store: &ContactStore, did: &DID) -> DID {
        let mut current = did.clone();
        let mut hops = 0;
        while let Some(next) = store.alias_index.get(&current) {
            if next == &current || hops > 32 {
                break;
            }
            current = next.clone();
            hops += 1;
        }
        current
    }

    pub async fn check_access_permission(
        &self,
        did: DID,
        context_id: Option<String>,
        owner: Option<DID>,
    ) -> std::result::Result<AccessDecision, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (decision, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;

            let decision = {
                let Some(contact) = store.contacts.get_mut(&did) else {
                    return Ok(AccessDecision {
                        level: AccessGroupLevel::Stranger,
                        allow_delivery: false,
                        target_box: "REQUEST_BOX".to_string(),
                        temporary_expires_at_ms: None,
                        reason: Some("contact not found; treated as stranger".to_string()),
                    });
                };

                let now_ms = Self::now_ms();
                Self::cleanup_expired_grants(contact, now_ms);

                match contact.access_level {
                    AccessGroupLevel::Block => AccessDecision {
                        level: AccessGroupLevel::Block,
                        allow_delivery: false,
                        target_box: "DROP".to_string(),
                        temporary_expires_at_ms: None,
                        reason: Some("contact is blocked".to_string()),
                    },
                    AccessGroupLevel::Friend => AccessDecision {
                        level: AccessGroupLevel::Friend,
                        allow_delivery: true,
                        target_box: "INBOX".to_string(),
                        temporary_expires_at_ms: None,
                        reason: None,
                    },
                    AccessGroupLevel::Temporary => {
                        let matched_grant = match context_id.as_ref() {
                            Some(context) => contact
                                .temp_grants
                                .iter()
                                .filter(|grant| grant.context_id == *context)
                                .max_by_key(|grant| grant.expires_at),
                            None => contact
                                .temp_grants
                                .iter()
                                .max_by_key(|grant| grant.expires_at),
                        };

                        if let Some(grant) = matched_grant {
                            AccessDecision {
                                level: AccessGroupLevel::Temporary,
                                allow_delivery: true,
                                target_box: "INBOX".to_string(),
                                temporary_expires_at_ms: Some(grant.expires_at),
                                reason: None,
                            }
                        } else {
                            contact.access_level = AccessGroupLevel::Stranger;
                            contact.updated_at = now_ms;
                            AccessDecision {
                                level: AccessGroupLevel::Stranger,
                                allow_delivery: false,
                                target_box: "REQUEST_BOX".to_string(),
                                temporary_expires_at_ms: None,
                                reason: Some("temporary grants expired".to_string()),
                            }
                        }
                    }
                    AccessGroupLevel::Stranger => AccessDecision {
                        level: AccessGroupLevel::Stranger,
                        allow_delivery: false,
                        target_box: "REQUEST_BOX".to_string(),
                        temporary_expires_at_ms: None,
                        reason: Some("contact is stranger".to_string()),
                    },
                }
            };

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (decision, store.clone())
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(decision)
    }

    pub async fn grant_temporary_access(
        &self,
        dids: Vec<DID>,
        context_id: String,
        duration_secs: u64,
        owner: Option<DID>,
    ) -> std::result::Result<GrantTemporaryAccessResult, RPCErrors> {
        let context_id = context_id.trim().to_string();
        if context_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "context_id is required".to_string(),
            ));
        }

        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (result, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;

            let now_ms = Self::now_ms();
            let expires_at = now_ms.saturating_add(duration_secs.saturating_mul(1000));

            let mut outcomes = Vec::with_capacity(dids.len());
            for did in dids {
                let contact = Self::ensure_contact_exists(
                    store,
                    did.clone(),
                    now_ms,
                    ContactSource::AutoInferred,
                );

                if contact.access_level == AccessGroupLevel::Block {
                    outcomes.push(TemporaryGrantOutcome {
                        did,
                        granted: false,
                        expires_at_ms: None,
                        reason: Some("contact is blocked".to_string()),
                    });
                    continue;
                }

                Self::cleanup_expired_grants(contact, now_ms);
                if let Some(grant) = contact
                    .temp_grants
                    .iter_mut()
                    .find(|grant| grant.context_id == context_id)
                {
                    grant.granted_at = now_ms;
                    grant.expires_at = expires_at;
                } else {
                    contact.temp_grants.push(TemporaryGrant {
                        context_id: context_id.clone(),
                        granted_at: now_ms,
                        expires_at,
                    });
                }

                if contact.access_level != AccessGroupLevel::Friend {
                    contact.access_level = AccessGroupLevel::Temporary;
                }
                contact.updated_at = now_ms;

                outcomes.push(TemporaryGrantOutcome {
                    did,
                    granted: true,
                    expires_at_ms: Some(expires_at),
                    reason: None,
                });
            }

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (
                GrantTemporaryAccessResult { updated: outcomes },
                store.clone(),
            )
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(result)
    }

    pub async fn block_contact(
        &self,
        did: DID,
        reason: Option<String>,
        owner: Option<DID>,
    ) -> std::result::Result<(), RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let snapshot = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;

            let now_ms = Self::now_ms();
            let contact =
                Self::ensure_contact_exists(store, did, now_ms, ContactSource::ManualCreate);

            contact.access_level = AccessGroupLevel::Block;
            contact.temp_grants.clear();
            contact.updated_at = now_ms;

            if let Some(reason) = reason {
                let reason = reason.trim();
                if !reason.is_empty() {
                    let existing = contact.note.clone().unwrap_or_default();
                    contact.note = Some(if existing.is_empty() {
                        format!("blocked: {}", reason)
                    } else {
                        format!("{} | blocked: {}", existing, reason)
                    });
                }
            }

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            store.clone()
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(())
    }

    pub async fn import_contacts(
        &self,
        contacts: Vec<ImportContactEntry>,
        upgrade_to_friend: Option<bool>,
        owner: Option<DID>,
    ) -> std::result::Result<ImportReport, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        // Import may mint brand-new DIDs (requires a DID seq DB round-trip) so
        // we can't do all the work under a single write guard. Pre-compute the
        // list of fresh DIDs we need, paired with their seed binding, before
        // taking the write lock.
        let now_ms = Self::now_ms();
        let upgrade_to_friend = upgrade_to_friend.unwrap_or(true);

        // For each entry, decide if it's new, and if so which (platform,
        // account) seed to use. We also pre-generate DIDs for brand-new
        // entries to avoid awaiting under the lock.
        let mut prepared_entries: Vec<PreparedEntry> = Vec::with_capacity(contacts.len());
        {
            let stores = self.stores.read().await;
            let store = stores.get(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;

            for entry in contacts {
                let name = entry.name.trim().to_string();
                let prepared_bindings = Self::prepare_import_bindings(entry.bindings, now_ms);
                if name.is_empty() && prepared_bindings.is_empty() {
                    prepared_entries.push(PreparedEntry::Skipped);
                    continue;
                }

                let mut matched_dids: Vec<DID> = Vec::new();
                let mut seen_dids: HashSet<String> = HashSet::new();
                for binding in &prepared_bindings {
                    let key = Self::binding_key(&binding.platform, &binding.account_id);
                    if let Some(did) = store.binding_index.get(&key) {
                        let did_key = did.to_string();
                        if seen_dids.insert(did_key) {
                            matched_dids.push(did.clone());
                        }
                    }
                }

                if matched_dids.is_empty() {
                    let seed_platform = prepared_bindings
                        .first()
                        .map(|binding| binding.platform.as_str())
                        .unwrap_or("import")
                        .to_string();
                    let seed_account = prepared_bindings
                        .first()
                        .map(|binding| binding.account_id.as_str())
                        .unwrap_or(name.as_str())
                        .to_string();
                    prepared_entries.push(PreparedEntry::NewNeedsDid {
                        name,
                        avatar: entry.avatar,
                        note: entry.note,
                        groups: entry.groups,
                        tags: entry.tags,
                        bindings: prepared_bindings,
                        seed_platform,
                        seed_account,
                    });
                } else {
                    prepared_entries.push(PreparedEntry::Existing {
                        name,
                        avatar: entry.avatar,
                        note: entry.note,
                        groups: entry.groups,
                        tags: entry.tags,
                        bindings: prepared_bindings,
                        matched_dids,
                    });
                }
            }
        }

        // Pre-generate fresh DIDs outside the lock.
        let mut generated: Vec<Option<DID>> = Vec::with_capacity(prepared_entries.len());
        for entry in &prepared_entries {
            match entry {
                PreparedEntry::NewNeedsDid {
                    seed_platform,
                    seed_account,
                    ..
                } => {
                    let did = self
                        .generate_contact_did(owner.as_ref(), seed_platform, seed_account)
                        .await?;
                    generated.push(Some(did));
                }
                _ => generated.push(None),
            }
        }

        let (report, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;

            let mut report = ImportReport::default();

            for (prepared, generated_did) in prepared_entries.into_iter().zip(generated.into_iter())
            {
                let canonical_did = match prepared {
                    PreparedEntry::Skipped => {
                        report.skipped = report.skipped.saturating_add(1);
                        continue;
                    }
                    PreparedEntry::NewNeedsDid {
                        name,
                        avatar,
                        note,
                        groups,
                        tags,
                        bindings,
                        ..
                    } => {
                        let did = generated_did.expect("generated did for new entry");
                        let mut contact = Self::blank_contact_with_source(
                            did.clone(),
                            now_ms,
                            ContactSource::ManualImport,
                        );
                        contact.name = if name.is_empty() {
                            did.to_string()
                        } else {
                            name
                        };
                        contact.avatar = avatar;
                        contact.note = note;
                        contact.groups = Self::dedupe_strings(groups);
                        contact.tags = Self::dedupe_strings(tags);
                        contact.is_verified = true;
                        if upgrade_to_friend {
                            contact.access_level = AccessGroupLevel::Friend;
                        }

                        for binding in bindings {
                            Self::upsert_binding(&mut contact, binding.clone());
                            let key = Self::binding_key(&binding.platform, &binding.account_id);
                            store.binding_index.insert(key, did.clone());
                        }

                        store.contacts.insert(did.clone(), contact);
                        report.created = report.created.saturating_add(1);
                        did
                    }
                    PreparedEntry::Existing {
                        name,
                        avatar,
                        note,
                        groups,
                        tags,
                        bindings,
                        matched_dids,
                    } => {
                        let mut sorted = matched_dids;
                        sorted.sort_by_key(|did| {
                            store
                                .contacts
                                .get(did)
                                .map(|contact| contact.updated_at)
                                .unwrap_or(0)
                        });
                        sorted.reverse();

                        let canonical = sorted[0].clone();
                        for source in sorted.into_iter().skip(1) {
                            if source != canonical {
                                if Self::merge_contacts_in_store(store, &canonical, &source, now_ms)
                                    .is_ok()
                                {
                                    report.merged = report.merged.saturating_add(1);
                                }
                            }
                        }

                        let mut binding_index_updates = Vec::new();
                        if let Some(contact) = store.contacts.get_mut(&canonical) {
                            if contact.source == ContactSource::AutoInferred {
                                report.upgraded_shadow = report.upgraded_shadow.saturating_add(1);
                            }

                            if !name.is_empty() {
                                contact.name = name;
                            }
                            if let Some(avatar) = avatar {
                                contact.avatar = Some(avatar);
                            }
                            if let Some(note) = note {
                                contact.note = Some(note);
                            }

                            contact.source = ContactSource::ManualImport;
                            contact.is_verified = true;
                            if upgrade_to_friend && contact.access_level != AccessGroupLevel::Block
                            {
                                contact.access_level = AccessGroupLevel::Friend;
                            }
                            contact.groups = Self::merge_string_lists(&contact.groups, groups);
                            contact.tags = Self::merge_string_lists(&contact.tags, tags);

                            for binding in bindings {
                                let key = Self::binding_key(&binding.platform, &binding.account_id);
                                Self::upsert_binding(contact, binding);
                                binding_index_updates.push(key);
                            }

                            contact.updated_at = now_ms;
                        }
                        for key in binding_index_updates {
                            store.binding_index.insert(key, canonical.clone());
                        }

                        canonical
                    }
                };

                report.imported = report.imported.saturating_add(1);
                if !report
                    .affected_dids
                    .iter()
                    .any(|existing| existing == &canonical_did)
                {
                    report.affected_dids.push(canonical_did);
                }
            }

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (report, store.clone())
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(report)
    }

    pub async fn merge_contacts(
        &self,
        target_did: DID,
        source_did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<Contact, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (result, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let now_ms = Self::now_ms();
            let result = Self::merge_contacts_in_store(store, &target_did, &source_did, now_ms)?;
            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (result, store.clone())
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(result)
    }

    pub async fn update_contact(
        &self,
        did: DID,
        patch: ContactPatch,
        owner: Option<DID>,
    ) -> std::result::Result<Contact, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (result, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let now_ms = Self::now_ms();
            let contact = store.contacts.get_mut(&did).ok_or_else(|| {
                RPCErrors::ReasonError(format!("contact not found for did {}", did.to_string()))
            })?;

            if let Some(name) = patch.name {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    contact.name = trimmed.to_string();
                }
            }
            if let Some(avatar) = patch.avatar {
                let trimmed = avatar.trim();
                contact.avatar = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if let Some(note) = patch.note {
                let trimmed = note.trim();
                contact.note = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }

            if let Some(access_level) = patch.access_level {
                contact.access_level = access_level.clone();
                if access_level != AccessGroupLevel::Temporary {
                    contact.temp_grants.clear();
                }
            }

            if let Some(source) = patch.source {
                contact.source = source.clone();
                if patch.is_verified.is_none() {
                    contact.is_verified = source != ContactSource::AutoInferred;
                }
            }

            if let Some(is_verified) = patch.is_verified {
                contact.is_verified = is_verified;
            }

            if let Some(groups) = patch.groups {
                contact.groups = Self::dedupe_strings(groups);
            }

            if let Some(tags) = patch.tags {
                contact.tags = Self::dedupe_strings(tags);
            }

            contact.updated_at = now_ms;
            let result = contact.clone();
            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (result, store.clone())
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(result)
    }

    pub async fn get_contact(
        &self,
        did: DID,
        owner: Option<DID>,
    ) -> std::result::Result<Option<Contact>, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;
        Ok(store.contacts.get(&did).cloned())
    }

    pub async fn list_contacts(
        &self,
        query: ContactQuery,
        owner: Option<DID>,
    ) -> std::result::Result<Vec<Contact>, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;

        let mut contacts: Vec<Contact> = store.contacts.values().cloned().collect();
        if let Some(source) = query.source {
            contacts.retain(|contact| contact.source == source);
        }
        if let Some(access_level) = query.access_level {
            contacts.retain(|contact| contact.access_level == access_level);
        }

        if let Some(keyword) = query.keyword {
            let keyword = keyword.trim().to_ascii_lowercase();
            if !keyword.is_empty() {
                contacts.retain(|contact| Self::contact_matches_keyword(contact, &keyword));
            }
        }

        contacts.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| left.did.to_string().cmp(&right.did.to_string()))
        });

        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query
            .limit
            .unwrap_or(DEFAULT_CONTACT_LIST_LIMIT)
            .clamp(1, MAX_CONTACT_LIST_LIMIT);

        Ok(contacts.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn get_group_subscribers(
        &self,
        group_id: DID,
        limit: Option<usize>,
        offset: Option<u64>,
        owner: Option<DID>,
    ) -> std::result::Result<Vec<DID>, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;
        let stores = self.stores.read().await;
        let store = stores.get(&owner_key).ok_or_else(|| {
            RPCErrors::ReasonError("contact store missing after load".to_string())
        })?;

        let list = store
            .group_subscribers
            .get(&group_id)
            .cloned()
            .unwrap_or_default();

        let offset = offset.unwrap_or(0) as usize;
        let limit = limit
            .unwrap_or(DEFAULT_GROUP_SUBSCRIBER_LIMIT)
            .clamp(1, MAX_GROUP_SUBSCRIBER_LIMIT);

        Ok(list.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn set_group_subscribers(
        &self,
        group_id: DID,
        subscribers: Vec<DID>,
        owner: Option<DID>,
    ) -> std::result::Result<SetGroupSubscribersResult, RPCErrors> {
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (result, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let unique = Self::dedupe_dids(subscribers);
            store
                .group_subscribers
                .insert(group_id.clone(), unique.clone());

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (
                SetGroupSubscribersResult {
                    group_id,
                    subscriber_count: unique.len(),
                },
                store.clone(),
            )
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(result)
    }

    pub async fn upsert_zone_user_contacts(
        &self,
        contacts: Vec<ZoneUserContactSeed>,
        owner: Option<DID>,
    ) -> std::result::Result<usize, RPCErrors> {
        let owner_scope = owner
            .as_ref()
            .map(|did| did.to_string())
            .unwrap_or_else(|| SYSTEM_OWNER_SCOPE.to_string());
        let owner_key = Self::owner_key(owner.as_ref());
        self.ensure_store_loaded(&owner_key).await?;

        let (updated, snapshot) = {
            let mut stores = self.stores.write().await;
            let store = stores.get_mut(&owner_key).ok_or_else(|| {
                RPCErrors::ReasonError("contact store missing after load".to_string())
            })?;
            let now_ms = Self::now_ms();
            let mut updated = 0usize;

            for seed in contacts {
                let ZoneUserContactSeed {
                    did,
                    name,
                    note,
                    bindings,
                    groups,
                    tags,
                } = seed;

                let prepared_bindings = Self::prepare_import_bindings(bindings, now_ms);
                let binding_count = prepared_bindings.len();
                let created = !store.contacts.contains_key(&did);
                let contact =
                    Self::ensure_contact_exists(store, did.clone(), now_ms, ContactSource::Shared);

                let trimmed_name = name.trim();
                if !trimmed_name.is_empty() {
                    contact.name = trimmed_name.to_string();
                } else if contact.name.trim().is_empty() {
                    contact.name = did.to_string();
                }
                contact.source = ContactSource::Shared;
                contact.is_verified = true;
                if contact.access_level != AccessGroupLevel::Block {
                    contact.access_level = AccessGroupLevel::Friend;
                }
                contact.note = note
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .or(contact.note.clone());

                let mut merged_groups = groups;
                merged_groups.push("zone_user".to_string());
                contact.groups = Self::merge_string_lists(&contact.groups, merged_groups);

                let mut merged_tags = tags;
                merged_tags.push("zone_user".to_string());
                contact.tags = Self::merge_string_lists(&contact.tags, merged_tags);

                for binding in prepared_bindings {
                    Self::upsert_binding(contact, binding);
                }
                contact.updated_at = now_ms;
                updated = updated.saturating_add(1);
                if created {
                    info!(
                        "zone user contact added from system config scan: owner_scope={}, did={}, name={}, binding_count={}",
                        owner_scope,
                        did.to_string(),
                        contact.name,
                        binding_count
                    );
                }
            }

            store.binding_index = Self::rebuild_binding_index(&store.contacts);
            (updated, store.clone())
        };

        self.save_owner_store(&owner_key, &snapshot).await?;
        Ok(updated)
    }

    async fn ensure_store_loaded(&self, owner_key: &str) -> std::result::Result<(), RPCErrors> {
        {
            let stores = self.stores.read().await;
            if stores.contains_key(owner_key) {
                return Ok(());
            }
        }

        let loaded = self.load_owner_store(owner_key).await?;
        let mut stores = self.stores.write().await;
        // Re-check; another task may have loaded it concurrently.
        stores.entry(owner_key.to_string()).or_insert(loaded);
        Ok(())
    }

    async fn load_owner_store(
        &self,
        owner_key: &str,
    ) -> std::result::Result<ContactStore, RPCErrors> {
        let contact_sql = self.render_sql("SELECT did, payload FROM contacts WHERE owner_key = ?");
        let rows = sqlx::query(&contact_sql)
            .bind(owner_key.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to query contacts: {}", error))
            })?;

        let mut contacts = HashMap::new();
        for row in rows {
            let did_str: String = row.try_get("did").map_err(|error| {
                RPCErrors::ReasonError(format!("failed to decode contacts.did: {}", error))
            })?;
            let payload: String = row.try_get("payload").map_err(|error| {
                RPCErrors::ReasonError(format!("failed to decode contacts.payload: {}", error))
            })?;
            let did = Self::parse_did(&did_str, "contacts.did")?;
            let contact: Contact = serde_json::from_str(&payload).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to parse contact payload for {}: {}",
                    did_str, error
                ))
            })?;
            contacts.insert(did, contact);
        }

        let group_sql = self.render_sql(
            "SELECT group_did, subscribers_json FROM group_subscribers WHERE owner_key = ?",
        );
        let group_rows = sqlx::query(&group_sql)
            .bind(owner_key.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to query group subscribers: {}", error))
            })?;

        let mut group_subscribers = HashMap::new();
        for row in group_rows {
            let group_did_str: String = row.try_get("group_did").map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to decode group_subscribers.group_did: {}",
                    error
                ))
            })?;
            let subscribers_json: String = row.try_get("subscribers_json").map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to decode group_subscribers.subscribers_json: {}",
                    error
                ))
            })?;
            let group_did = Self::parse_did(&group_did_str, "group_subscribers.group_did")?;
            let did_strings: Vec<String> =
                serde_json::from_str(&subscribers_json).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "failed to parse group subscribers {}: {}",
                        group_did_str, error
                    ))
                })?;
            let mut dids = Vec::with_capacity(did_strings.len());
            for did in did_strings {
                dids.push(Self::parse_did(&did, "group_subscribers.subscriber")?);
            }
            group_subscribers.insert(group_did, Self::dedupe_dids(dids));
        }

        let alias_index = self.load_alias_index(owner_key).await?;

        Ok(ContactStore {
            binding_index: Self::rebuild_binding_index(&contacts),
            contacts,
            group_subscribers,
            alias_index,
        })
    }

    async fn load_alias_index(
        &self,
        owner_key: &str,
    ) -> std::result::Result<HashMap<DID, DID>, RPCErrors> {
        let key = format!("{}{}", METADATA_ALIAS_INDEX_PREFIX, owner_key);
        let sql = self.render_sql("SELECT value FROM contact_metadata WHERE key = ?");
        let row = sqlx::query(&sql)
            .bind(key)
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to query alias index: {}", error))
            })?;

        let Some(row) = row else {
            return Ok(HashMap::new());
        };
        let value: String = row.try_get("value").map_err(|error| {
            RPCErrors::ReasonError(format!("failed to decode alias index value: {}", error))
        })?;
        let raw: HashMap<String, String> = serde_json::from_str(&value).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to parse alias index: {}", error))
        })?;
        let mut alias_index = HashMap::with_capacity(raw.len());
        for (alias, canonical) in raw {
            let alias = Self::parse_did(&alias, "alias_index.alias")?;
            let canonical = Self::parse_did(&canonical, "alias_index.canonical")?;
            alias_index.insert(alias, canonical);
        }
        Ok(alias_index)
    }

    async fn save_owner_store(
        &self,
        owner_key: &str,
        store: &ContactStore,
    ) -> std::result::Result<(), RPCErrors> {
        let mut tx = self.pool().begin().await.map_err(|error| {
            RPCErrors::ReasonError(format!("failed to begin save store tx: {}", error))
        })?;

        let delete_contacts = self.render_sql("DELETE FROM contacts WHERE owner_key = ?");
        sqlx::query(&delete_contacts)
            .bind(owner_key.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to clear contacts: {}", error))
            })?;

        let mut contacts: Vec<&Contact> = store.contacts.values().collect();
        contacts.sort_by(|left, right| left.did.to_string().cmp(&right.did.to_string()));
        let insert_contact =
            self.render_sql("INSERT INTO contacts (owner_key, did, payload) VALUES (?, ?, ?)");
        for contact in contacts {
            let payload = serde_json::to_string(contact).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to serialize contact {}: {}",
                    contact.did.to_string(),
                    error
                ))
            })?;

            sqlx::query(&insert_contact)
                .bind(owner_key.to_string())
                .bind(contact.did.to_string())
                .bind(payload)
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "failed to persist contact {}: {}",
                        contact.did.to_string(),
                        error
                    ))
                })?;
        }

        let delete_groups = self.render_sql("DELETE FROM group_subscribers WHERE owner_key = ?");
        sqlx::query(&delete_groups)
            .bind(owner_key.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to clear group subscribers: {}", error))
            })?;

        let mut groups: Vec<(&DID, &Vec<DID>)> = store.group_subscribers.iter().collect();
        groups.sort_by(|left, right| left.0.to_string().cmp(&right.0.to_string()));

        let insert_group = self.render_sql(
            "INSERT INTO group_subscribers (owner_key, group_did, subscribers_json) VALUES (?, ?, ?)",
        );
        for (group_id, subscribers) in groups {
            let payload = serde_json::to_string(
                &subscribers
                    .iter()
                    .map(|did| did.to_string())
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to serialize subscribers for {}: {}",
                    group_id.to_string(),
                    error
                ))
            })?;

            sqlx::query(&insert_group)
                .bind(owner_key.to_string())
                .bind(group_id.to_string())
                .bind(payload)
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "failed to persist group subscribers for {}: {}",
                        group_id.to_string(),
                        error
                    ))
                })?;
        }

        let alias_key = format!("{}{}", METADATA_ALIAS_INDEX_PREFIX, owner_key);
        let alias_raw: HashMap<String, String> = store
            .alias_index
            .iter()
            .map(|(alias, canonical)| (alias.to_string(), canonical.to_string()))
            .collect();
        let alias_payload = serde_json::to_string(&alias_raw).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to serialize alias index: {}", error))
        })?;
        let upsert_alias = self.render_sql(
            "INSERT INTO contact_metadata(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        );
        sqlx::query(&upsert_alias)
            .bind(alias_key)
            .bind(alias_payload)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to persist alias index: {}", error))
            })?;

        tx.commit().await.map_err(|error| {
            RPCErrors::ReasonError(format!("failed to commit contact store tx: {}", error))
        })?;
        Ok(())
    }

    async fn load_next_did_seq(&self) -> std::result::Result<u64, RPCErrors> {
        let sql = self.render_sql("SELECT value FROM contact_metadata WHERE key = ?");
        let row = sqlx::query(&sql)
            .bind(METADATA_DID_SEQ_KEY.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to query did sequence: {}", error))
            })?;

        match row {
            Some(row) => {
                let value: String = row.try_get("value").map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "failed to decode did sequence value: {}",
                        error
                    ))
                })?;
                value.parse::<u64>().map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "invalid did sequence value '{}': {}",
                        value, error
                    ))
                })
            }
            None => {
                self.persist_next_did_seq(1).await?;
                Ok(1)
            }
        }
    }

    async fn persist_next_did_seq(&self, next_seq: u64) -> std::result::Result<(), RPCErrors> {
        let sql = self.render_sql(
            "INSERT INTO contact_metadata(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        );
        sqlx::query(&sql)
            .bind(METADATA_DID_SEQ_KEY.to_string())
            .bind(next_seq.to_string())
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to persist did sequence: {}", error))
            })?;
        Ok(())
    }

    async fn next_did_seq(&self) -> std::result::Result<u64, RPCErrors> {
        let seq = self.did_seq.fetch_add(1, Ordering::SeqCst);
        let next = seq.saturating_add(1);
        self.persist_next_did_seq(next).await?;
        Ok(seq)
    }

    fn owner_key(owner: Option<&DID>) -> String {
        owner
            .map(|did| did.to_string())
            .unwrap_or_else(|| SYSTEM_OWNER_SCOPE.to_string())
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn binding_key(platform: &str, account_id: &str) -> String {
        format!(
            "{}:{}",
            platform.trim().to_ascii_lowercase(),
            account_id.trim().to_ascii_lowercase()
        )
    }

    fn rebuild_binding_index(contacts: &HashMap<DID, Contact>) -> HashMap<String, DID> {
        let mut weighted: HashMap<String, (u8, u64, DID)> = HashMap::new();
        for contact in contacts.values() {
            let candidate_priority = Self::binding_priority(contact);
            for binding in &contact.bindings {
                let key = Self::binding_key(&binding.platform, &binding.account_id);
                match weighted.get(&key) {
                    Some((current_priority, current_updated_at, _))
                        if *current_priority > candidate_priority
                            || (*current_priority == candidate_priority
                                && *current_updated_at > contact.updated_at) => {}
                    _ => {
                        weighted.insert(
                            key,
                            (candidate_priority, contact.updated_at, contact.did.clone()),
                        );
                    }
                }
            }
        }

        weighted
            .into_iter()
            .map(|(key, (_, _, did))| (key, did))
            .collect()
    }

    fn binding_priority(contact: &Contact) -> u8 {
        let is_zone_user = contact
            .groups
            .iter()
            .any(|group| group.eq_ignore_ascii_case("zone_user"));
        if is_zone_user {
            return 3;
        }
        match contact.source {
            ContactSource::ManualImport | ContactSource::ManualCreate => 2,
            ContactSource::Shared | ContactSource::AutoInferred => 1,
        }
    }

    fn parse_did(raw: &str, field: &str) -> std::result::Result<DID, RPCErrors> {
        DID::from_str(raw).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to parse {} '{}': {}", field, raw, error))
        })
    }

    fn sanitize_subject(raw: &str) -> String {
        let mut result = String::with_capacity(raw.len());
        let mut previous_dash = false;
        for ch in raw.chars() {
            let mapped = if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else {
                Some('-')
            };

            if let Some(mapped) = mapped {
                if mapped == '-' {
                    if !previous_dash {
                        result.push(mapped);
                    }
                    previous_dash = true;
                } else {
                    result.push(mapped);
                    previous_dash = false;
                }
            }
        }

        let trimmed = result.trim_matches('-').to_string();
        if trimmed.is_empty() {
            "contact".to_string()
        } else {
            trimmed.chars().take(48).collect()
        }
    }

    async fn generate_contact_did(
        &self,
        owner: Option<&DID>,
        platform: &str,
        account_id: &str,
    ) -> std::result::Result<DID, RPCErrors> {
        let seq = self.next_did_seq().await?;
        let owner_seed = owner
            .map(|did| did.to_raw_host_name())
            .unwrap_or_else(|| SYSTEM_OWNER_SCOPE.to_string());

        let subject = format!(
            "mc-{}-{}-{}-{}",
            Self::sanitize_subject(&owner_seed),
            Self::sanitize_subject(platform),
            Self::sanitize_subject(account_id),
            seq
        );
        Ok(DID::new("bns", &subject))
    }

    fn message_tunnel_endpoint_did(
        account_id: &str,
        profile_hint: Option<&Value>,
    ) -> std::result::Result<Option<MessageTunnelEndpointDid>, RPCErrors> {
        let Some(account_type) = Self::extract_hint_string(profile_hint, &["account_type"]) else {
            return Ok(None);
        };
        let Some(tunnel_instance_id) =
            Self::extract_hint_string(profile_hint, &["tunnel_instance_id", "tunnel"])
        else {
            return Ok(None);
        };

        let account_type = Self::normalize_account_type(&account_type);
        if account_type.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "account_type is required for message tunnel endpoint did".to_string(),
            ));
        }
        let tunnel_instance_id = tunnel_instance_id.trim().to_string();
        if tunnel_instance_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "tunnel_instance_id is required for message tunnel endpoint did".to_string(),
            ));
        }

        let encoded_account_id = Self::percent_encode_did_component(account_id);
        let encoded_tunnel_id = Self::percent_encode_did_component(&tunnel_instance_id);
        let subject = format!(
            "{}.{}.{}",
            encoded_account_id, account_type, encoded_tunnel_id
        );
        Ok(Some(MessageTunnelEndpointDid {
            did: DID::new("msgtunnel", &subject),
            account_type,
            tunnel_instance_id,
            encoded_account_id,
        }))
    }

    fn build_endpoint_binding(
        platform: String,
        account_id: String,
        profile_hint: Option<&Value>,
        now_ms: u64,
        endpoint: &MessageTunnelEndpointDid,
        did: &DID,
    ) -> AccountBinding {
        let display_id = Self::extract_hint_string(profile_hint, &["display_id", "username"])
            .unwrap_or_else(|| account_id.clone());
        let tunnel_instance_id =
            Self::extract_hint_string(profile_hint, &["tunnel_instance_id", "tunnel"])
                .unwrap_or_else(|| endpoint.tunnel_instance_id.clone());
        let mut binding = AccountBinding {
            platform,
            account_id,
            display_id,
            tunnel_instance_id,
            account_type: endpoint.account_type.clone(),
            endpoint_did: Some(did.clone()),
            last_active_at: now_ms,
            meta: HashMap::new(),
        };
        Self::merge_hint_meta(&mut binding, profile_hint);
        Self::merge_endpoint_meta(&mut binding, endpoint, did);
        binding
    }

    fn merge_endpoint_meta(
        binding: &mut AccountBinding,
        endpoint: &MessageTunnelEndpointDid,
        did: &DID,
    ) {
        // Promote the endpoint identity onto the typed binding fields so callers
        // do not have to read `meta`. `tunnel_instance_id` stays the stable short id.
        binding.account_type = endpoint.account_type.clone();
        binding.endpoint_did = Some(did.clone());
        binding.tunnel_instance_id = endpoint.tunnel_instance_id.clone();
        binding
            .meta
            .insert("account_type".to_string(), endpoint.account_type.clone());
        binding
            .meta
            .insert("message_tunnel_did".to_string(), did.to_string());
        binding.meta.insert(
            "message_tunnel_encoded_account_id".to_string(),
            endpoint.encoded_account_id.clone(),
        );
        binding.meta.insert(
            "message_tunnel_id".to_string(),
            endpoint.tunnel_instance_id.clone(),
        );
    }

    fn normalize_account_type(raw: &str) -> String {
        raw.trim()
            .chars()
            .filter_map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    Some(ch.to_ascii_lowercase())
                } else if ch == '_' || ch == '-' {
                    Some(ch)
                } else {
                    None
                }
            })
            .collect()
    }

    fn percent_encode_did_component(raw: &str) -> String {
        let mut output = String::with_capacity(raw.len());
        for byte in raw.as_bytes() {
            if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'~') {
                output.push(*byte as char);
            } else {
                output.push_str(&format!("%{:02X}", byte));
            }
        }
        output
    }

    /// Reverse of `percent_encode_did_component`.
    pub fn decode_did_component(raw: &str) -> String {
        let bytes = raw.as_bytes();
        let mut output: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    output.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
            }
            output.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&output).into_owned()
    }

    /// Decompose a second-level endpoint DID
    /// `did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_instance_id>`.
    /// Returns `(account_id, account_type, tunnel_instance_id)` with `account_id` /
    /// `tunnel_instance_id` percent-decoded. `None` if `did` is not a well-formed
    /// `did:msgtunnel:*`.
    pub fn parse_msgtunnel_did(did: &DID) -> Option<(String, String, String)> {
        if did.method != "msgtunnel" {
            return None;
        }
        let parts: Vec<&str> = did.id.splitn(3, '.').collect();
        if parts.len() != 3 {
            return None;
        }
        let account_id = Self::decode_did_component(parts[0]);
        let account_type = parts[1].to_string();
        let tunnel_instance_id = Self::decode_did_component(parts[2]);
        if account_id.is_empty() || account_type.is_empty() || tunnel_instance_id.is_empty() {
            return None;
        }
        Some((account_id, account_type, tunnel_instance_id))
    }

    fn blank_contact_with_source(did: DID, now_ms: u64, source: ContactSource) -> Contact {
        Contact {
            did,
            name: "".to_string(),
            avatar: None,
            note: None,
            source: source.clone(),
            is_verified: source != ContactSource::AutoInferred,
            bindings: Vec::new(),
            access_level: AccessGroupLevel::Stranger,
            temp_grants: Vec::new(),
            groups: Vec::new(),
            tags: Vec::new(),
            created_at: now_ms,
            updated_at: now_ms,
        }
    }

    fn build_shadow_contact(
        did: DID,
        platform: String,
        account_id: String,
        profile_hint: Option<&Value>,
        now_ms: u64,
    ) -> Contact {
        let name = Self::extract_hint_string(
            profile_hint,
            &["name", "display_name", "nickname", "full_name"],
        )
        .unwrap_or_else(|| format!("{}:{}", platform, account_id));
        let avatar = Self::extract_hint_string(profile_hint, &["avatar", "avatar_url"]);
        let note = Self::extract_hint_string(profile_hint, &["note", "bio", "desc"]);
        let display_id = Self::extract_hint_string(profile_hint, &["display_id", "username"])
            .unwrap_or_else(|| account_id.clone());
        let tunnel_instance_id =
            Self::extract_hint_string(profile_hint, &["tunnel_instance_id", "tunnel"])
                .unwrap_or_else(|| format!("{}-default", platform.to_ascii_lowercase()));
        let account_type = Self::extract_hint_string(profile_hint, &["account_type"])
            .map(|raw| Self::normalize_account_type(&raw))
            .unwrap_or_default();

        let mut binding = AccountBinding {
            platform,
            account_id,
            display_id,
            tunnel_instance_id,
            account_type,
            endpoint_did: None,
            last_active_at: now_ms,
            meta: HashMap::new(),
        };
        Self::merge_hint_meta(&mut binding, profile_hint);

        Contact {
            did,
            name,
            avatar,
            note,
            source: ContactSource::AutoInferred,
            is_verified: false,
            bindings: vec![binding],
            access_level: AccessGroupLevel::Stranger,
            temp_grants: Vec::new(),
            groups: Vec::new(),
            tags: Vec::new(),
            created_at: now_ms,
            updated_at: now_ms,
        }
    }

    fn ensure_contact_exists<'a>(
        store: &'a mut ContactStore,
        did: DID,
        now_ms: u64,
        source: ContactSource,
    ) -> &'a mut Contact {
        store
            .contacts
            .entry(did.clone())
            .or_insert_with(|| Contact {
                did: did.clone(),
                name: did.to_string(),
                avatar: None,
                note: None,
                source: source.clone(),
                is_verified: source != ContactSource::AutoInferred,
                bindings: Vec::new(),
                access_level: AccessGroupLevel::Stranger,
                temp_grants: Vec::new(),
                groups: Vec::new(),
                tags: Vec::new(),
                created_at: now_ms,
                updated_at: now_ms,
            })
    }

    fn cleanup_expired_grants(contact: &mut Contact, now_ms: u64) {
        contact
            .temp_grants
            .retain(|grant| grant.expires_at > now_ms);
    }

    fn prepare_import_bindings(bindings: Vec<AccountBinding>, now_ms: u64) -> Vec<AccountBinding> {
        let mut prepared = Vec::new();
        let mut seen = HashSet::new();

        for mut binding in bindings {
            binding.platform = binding.platform.trim().to_string();
            binding.account_id = binding.account_id.trim().to_string();
            if binding.platform.is_empty() || binding.account_id.is_empty() {
                continue;
            }

            if binding.display_id.trim().is_empty() {
                binding.display_id = binding.account_id.clone();
            }
            if binding.tunnel_instance_id.trim().is_empty() {
                binding.tunnel_instance_id =
                    format!("{}-default", binding.platform.to_ascii_lowercase());
            }
            if binding.last_active_at == 0 {
                binding.last_active_at = now_ms;
            }

            let key = Self::binding_key(&binding.platform, &binding.account_id);
            if seen.insert(key) {
                prepared.push(binding);
            }
        }

        prepared
    }

    fn find_binding_mut<'a>(
        contact: &'a mut Contact,
        platform: &str,
        account_id: &str,
    ) -> Option<&'a mut AccountBinding> {
        contact.bindings.iter_mut().find(|binding| {
            binding.platform.eq_ignore_ascii_case(platform)
                && binding.account_id.eq_ignore_ascii_case(account_id)
        })
    }

    fn upsert_binding(contact: &mut Contact, binding: AccountBinding) {
        if let Some(existing) =
            Self::find_binding_mut(contact, &binding.platform, &binding.account_id)
        {
            if binding.last_active_at >= existing.last_active_at {
                existing.last_active_at = binding.last_active_at;
                existing.display_id = binding.display_id;
                existing.tunnel_instance_id = binding.tunnel_instance_id;
            }
            if !binding.account_type.is_empty() {
                existing.account_type = binding.account_type;
            }
            if binding.endpoint_did.is_some() {
                existing.endpoint_did = binding.endpoint_did;
            }
            for (key, value) in binding.meta {
                existing.meta.insert(key, value);
            }
            return;
        }

        contact.bindings.push(binding);
    }

    fn merge_hint_meta(binding: &mut AccountBinding, profile_hint: Option<&Value>) {
        let Some(Value::Object(object)) = profile_hint else {
            return;
        };

        if let Some(Value::Object(meta)) = object.get("meta") {
            for (key, value) in meta {
                if let Some(string) = Self::value_to_flat_string(value) {
                    binding.meta.insert(key.clone(), string);
                }
            }
        }

        if let Some(platform_uid) = object
            .get("platform_uid")
            .and_then(Self::value_to_flat_string)
        {
            binding
                .meta
                .insert("platform_uid".to_string(), platform_uid);
        }

        for key in [
            "account_type",
            "chat_type",
            "chat_id",
            "bot_account_id",
            "message_tunnel_did",
            "message_tunnel_encoded_account_id",
            "message_tunnel_id",
        ] {
            if let Some(value) = object.get(key).and_then(Self::value_to_flat_string) {
                binding.meta.insert(key.to_string(), value);
            }
        }
    }

    fn extract_hint_string(profile_hint: Option<&Value>, keys: &[&str]) -> Option<String> {
        let Value::Object(object) = profile_hint? else {
            return None;
        };

        for key in keys {
            if let Some(value) = object.get(*key).and_then(Self::value_to_flat_string) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }

        None
    }

    fn value_to_flat_string(value: &Value) -> Option<String> {
        match value {
            Value::String(value) => Some(value.clone()),
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        }
    }

    fn merge_contacts_in_store(
        store: &mut ContactStore,
        target_did: &DID,
        source_did: &DID,
        now_ms: u64,
    ) -> std::result::Result<Contact, RPCErrors> {
        if target_did == source_did {
            return Err(RPCErrors::ParseRequestError(
                "target_did and source_did must be different".to_string(),
            ));
        }

        if !store.contacts.contains_key(target_did) {
            return Err(RPCErrors::ReasonError(format!(
                "target contact not found for did {}",
                target_did.to_string()
            )));
        }

        let source = store.contacts.remove(source_did).ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "source contact not found for did {}",
                source_did.to_string()
            ))
        })?;

        let (target_snapshot, target_bindings) = {
            let target = store.contacts.get_mut(target_did).ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "target contact not found for did {}",
                    target_did.to_string()
                ))
            })?;

            if target.name.trim().is_empty() {
                target.name = source.name.clone();
            }
            if target.avatar.is_none() {
                target.avatar = source.avatar.clone();
            }
            if target.note.is_none() {
                target.note = source.note.clone();
            }

            target.source = Self::choose_more_reliable_source(target.source.clone(), source.source);
            target.is_verified = target.is_verified || source.is_verified;
            target.access_level =
                Self::merge_access_levels(target.access_level.clone(), source.access_level);
            target.temp_grants = Self::merge_grants(target.temp_grants.clone(), source.temp_grants);
            target.groups = Self::merge_string_lists(&target.groups, source.groups);
            target.tags = Self::merge_string_lists(&target.tags, source.tags);

            for binding in source.bindings {
                Self::upsert_binding(target, binding);
            }

            Self::cleanup_expired_grants(target, now_ms);
            target.updated_at = now_ms;

            (target.clone(), target.bindings.clone())
        };

        store
            .binding_index
            .retain(|_, did| did != source_did && did != target_did);
        for binding in &target_bindings {
            let key = Self::binding_key(&binding.platform, &binding.account_id);
            store.binding_index.insert(key, target_did.clone());
        }

        for subscribers in store.group_subscribers.values_mut() {
            for subscriber in subscribers.iter_mut() {
                if subscriber == source_did {
                    *subscriber = target_did.clone();
                }
            }
            *subscribers = Self::dedupe_dids(std::mem::take(subscribers));
        }

        // Tombstone the source: keep it as an alias of the target so old
        // MsgObjects / session peer DIDs still resolve. Do NOT delete it.
        // Re-point any existing alias that pointed at the source.
        for canonical in store.alias_index.values_mut() {
            if canonical == source_did {
                *canonical = target_did.clone();
            }
        }
        store
            .alias_index
            .insert(source_did.clone(), target_did.clone());
        // The target is canonical; it must never appear as an alias key.
        store.alias_index.remove(target_did);

        Ok(target_snapshot)
    }

    fn choose_more_reliable_source(left: ContactSource, right: ContactSource) -> ContactSource {
        fn weight(source: &ContactSource) -> u8 {
            match source {
                ContactSource::ManualImport => 4,
                ContactSource::ManualCreate => 3,
                ContactSource::Shared => 2,
                ContactSource::AutoInferred => 1,
            }
        }

        if weight(&left) >= weight(&right) {
            left
        } else {
            right
        }
    }

    fn merge_access_levels(left: AccessGroupLevel, right: AccessGroupLevel) -> AccessGroupLevel {
        fn rank(level: &AccessGroupLevel) -> u8 {
            match level {
                AccessGroupLevel::Block => 4,
                AccessGroupLevel::Friend => 3,
                AccessGroupLevel::Temporary => 2,
                AccessGroupLevel::Stranger => 1,
            }
        }

        if rank(&left) >= rank(&right) {
            left
        } else {
            right
        }
    }

    fn merge_grants(left: Vec<TemporaryGrant>, right: Vec<TemporaryGrant>) -> Vec<TemporaryGrant> {
        let mut grants = HashMap::<String, TemporaryGrant>::new();
        for grant in left.into_iter().chain(right) {
            match grants.get(&grant.context_id) {
                Some(existing) if existing.expires_at >= grant.expires_at => {}
                _ => {
                    grants.insert(grant.context_id.clone(), grant);
                }
            }
        }
        grants.into_values().collect()
    }

    fn dedupe_strings(values: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            let key = trimmed.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(trimmed.to_string());
            }
        }
        out
    }

    fn merge_string_lists(left: &[String], right: Vec<String>) -> Vec<String> {
        let mut merged = left.to_vec();
        merged.extend(right);
        Self::dedupe_strings(merged)
    }

    fn dedupe_dids(values: Vec<DID>) -> Vec<DID> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for did in values {
            let key = did.to_string();
            if seen.insert(key) {
                out.push(did);
            }
        }
        out
    }

    fn contact_matches_keyword(contact: &Contact, keyword: &str) -> bool {
        if contact
            .did
            .to_string()
            .to_ascii_lowercase()
            .contains(keyword)
        {
            return true;
        }
        if contact.name.to_ascii_lowercase().contains(keyword) {
            return true;
        }
        if contact
            .note
            .as_ref()
            .map(|note| note.to_ascii_lowercase().contains(keyword))
            .unwrap_or(false)
        {
            return true;
        }
        if contact
            .groups
            .iter()
            .any(|group| group.to_ascii_lowercase().contains(keyword))
        {
            return true;
        }
        if contact
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(keyword))
        {
            return true;
        }

        contact.bindings.iter().any(|binding| {
            binding.platform.to_ascii_lowercase().contains(keyword)
                || binding.account_id.to_ascii_lowercase().contains(keyword)
                || binding.display_id.to_ascii_lowercase().contains(keyword)
                || binding
                    .tunnel_instance_id
                    .to_ascii_lowercase()
                    .contains(keyword)
                || binding.meta.iter().any(|(key, value)| {
                    key.to_ascii_lowercase().contains(keyword)
                        || value.to_ascii_lowercase().contains(keyword)
                })
        })
    }
}

enum PreparedEntry {
    Skipped,
    NewNeedsDid {
        name: String,
        avatar: Option<String>,
        note: Option<String>,
        groups: Vec<String>,
        tags: Vec<String>,
        bindings: Vec<AccountBinding>,
        seed_platform: String,
        seed_account: String,
    },
    Existing {
        name: String,
        avatar: Option<String>,
        note: Option<String>,
        groups: Vec<String>,
        tags: Vec<String>,
        bindings: Vec<AccountBinding>,
        matched_dids: Vec<DID>,
    },
}

fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn binding(platform: &str, account_id: &str) -> AccountBinding {
        AccountBinding {
            platform: platform.to_string(),
            account_id: account_id.to_string(),
            display_id: account_id.to_string(),
            tunnel_instance_id: format!("{}-default", platform),
            account_type: String::new(),
            endpoint_did: None,
            last_active_at: 1,
            meta: HashMap::new(),
        }
    }

    async fn new_test_mgr() -> (ContactMgr, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("contacts.sqlite3");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let msg_box_db = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
        let mgr = ContactMgr::new_with_msg_box(msg_box_db).await.unwrap();
        (mgr, temp_dir)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_did_reuses_existing_binding() {
        let (mgr, _tmp) = new_test_mgr().await;

        let first = mgr
            .resolve_did("telegram".to_string(), "12345".to_string(), None, None)
            .await
            .unwrap();
        let second = mgr
            .resolve_did("telegram".to_string(), "12345".to_string(), None, None)
            .await
            .unwrap();

        assert_eq!(first, second);

        let contact = mgr.get_contact(first.clone(), None).await.unwrap().unwrap();
        assert_eq!(contact.source, ContactSource::AutoInferred);
        assert_eq!(contact.access_level, AccessGroupLevel::Stranger);
        assert_eq!(contact.bindings.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_did_uses_message_tunnel_endpoint_did_when_hint_is_present() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("web", "jarvis.test.buckyos.io");
        let profile_hint = json!({
            "account_type": "user",
            "tunnel_instance_id": "did:web:tg-tunnel.test.buckyos.io",
            "display_id": "@alice",
            "bot_account_id": "@jarvis_bot"
        });

        let first = mgr
            .resolve_did(
                "telegram".to_string(),
                "alice.id/with:chars".to_string(),
                Some(profile_hint.clone()),
                Some(owner.clone()),
            )
            .await
            .unwrap();
        let second = mgr
            .resolve_did(
                "telegram".to_string(),
                "alice.id/with:chars".to_string(),
                Some(profile_hint),
                Some(owner.clone()),
            )
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first.to_string(),
            "did:msgtunnel:alice%2Eid%2Fwith%3Achars.user.did%3Aweb%3Atg-tunnel%2Etest%2Ebuckyos%2Eio"
        );

        let contact = mgr
            .get_contact(first.clone(), Some(owner))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(contact.source, ContactSource::AutoInferred);
        assert_eq!(contact.bindings.len(), 1);
        assert_eq!(contact.bindings[0].platform, "telegram");
        assert_eq!(contact.bindings[0].account_id, "alice.id/with:chars");
        assert_eq!(
            contact.bindings[0]
                .meta
                .get("account_type")
                .map(String::as_str),
            Some("user")
        );
        assert_eq!(
            contact.bindings[0]
                .meta
                .get("message_tunnel_did")
                .map(String::as_str),
            Some(first.to_string().as_str())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn message_tunnel_endpoint_did_does_not_override_primary_contact_binding() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("web", "jarvis.test.buckyos.io");
        let zone_user_did = DID::new("bns", "alice");

        mgr.upsert_zone_user_contacts(
            vec![ZoneUserContactSeed {
                did: zone_user_did.clone(),
                name: "Alice".to_string(),
                note: None,
                bindings: vec![binding("telegram", "12345")],
                groups: vec!["zone_user".to_string()],
                tags: vec![],
            }],
            Some(owner.clone()),
        )
        .await
        .unwrap();

        let primary = mgr
            .resolve_did(
                "telegram".to_string(),
                "12345".to_string(),
                None,
                Some(owner.clone()),
            )
            .await
            .unwrap();
        let endpoint = mgr
            .resolve_did(
                "telegram".to_string(),
                "12345".to_string(),
                Some(json!({
                    "account_type": "user",
                    "tunnel_instance_id": "did:web:tg-tunnel.test.buckyos.io"
                })),
                Some(owner),
            )
            .await
            .unwrap();

        assert_eq!(primary, zone_user_did);
        assert_eq!(
            endpoint.to_string(),
            "did:msgtunnel:12345.user.did%3Aweb%3Atg-tunnel%2Etest%2Ebuckyos%2Eio"
        );
        assert_ne!(endpoint, zone_user_did);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn temporary_access_expires_and_downgrades() {
        let (mgr, _tmp) = new_test_mgr().await;
        let did = DID::new("bns", "alice-temp");

        mgr.grant_temporary_access(vec![did.clone()], "ctx-a".to_string(), 0, None)
            .await
            .unwrap();

        let decision = mgr
            .check_access_permission(did.clone(), Some("ctx-a".to_string()), None)
            .await
            .unwrap();
        assert_eq!(decision.level, AccessGroupLevel::Stranger);
        assert!(!decision.allow_delivery);
        assert_eq!(decision.target_box, "REQUEST_BOX");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn import_upgrades_shadow_contact() {
        let (mgr, _tmp) = new_test_mgr().await;

        let did = mgr
            .resolve_did("telegram".to_string(), "user-1".to_string(), None, None)
            .await
            .unwrap();

        let report = mgr
            .import_contacts(
                vec![ImportContactEntry {
                    name: "Alice".to_string(),
                    avatar: None,
                    note: Some("friend".to_string()),
                    bindings: vec![binding("telegram", "user-1")],
                    groups: vec!["team".to_string()],
                    tags: vec!["vip".to_string()],
                }],
                Some(true),
                None,
            )
            .await
            .unwrap();

        assert_eq!(report.imported, 1);
        assert_eq!(report.upgraded_shadow, 1);

        let contact = mgr.get_contact(did, None).await.unwrap().unwrap();
        assert_eq!(contact.name, "Alice");
        assert_eq!(contact.source, ContactSource::ManualImport);
        assert_eq!(contact.access_level, AccessGroupLevel::Friend);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn merge_contacts_moves_bindings() {
        let (mgr, _tmp) = new_test_mgr().await;

        let target = mgr
            .resolve_did(
                "telegram".to_string(),
                "merge-target".to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        let source = mgr
            .resolve_did(
                "email".to_string(),
                "merge-source@example.com".to_string(),
                None,
                None,
            )
            .await
            .unwrap();

        mgr.grant_temporary_access(vec![source.clone()], "ctx-merge".to_string(), 60, None)
            .await
            .unwrap();

        let merged = mgr
            .merge_contacts(target.clone(), source.clone(), None)
            .await
            .unwrap();

        assert_eq!(merged.did, target);
        assert!(mgr
            .get_contact(source.clone(), None)
            .await
            .unwrap()
            .is_none());

        let resolved = mgr
            .resolve_did(
                "email".to_string(),
                "merge-source@example.com".to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn parse_msgtunnel_did_round_trips() {
        // Plain numeric id.
        let did = DID::new("msgtunnel", "12345.user.tg-main");
        assert_eq!(
            ContactMgr::parse_msgtunnel_did(&did),
            Some((
                "12345".to_string(),
                "user".to_string(),
                "tg-main".to_string()
            ))
        );

        // Special chars in the account id survive percent encode/decode.
        let encoded = ContactMgr::percent_encode_did_component("bob@example.com");
        let did = DID::new("msgtunnel", &format!("{}.addr.email-main", encoded));
        assert_eq!(
            ContactMgr::parse_msgtunnel_did(&did),
            Some((
                "bob@example.com".to_string(),
                "addr".to_string(),
                "email-main".to_string()
            ))
        );

        // Non-tunnel DIDs are rejected.
        assert!(ContactMgr::parse_msgtunnel_did(&DID::new("bns", "bob")).is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn telegram_endpoint_dids_are_stable_short_id_and_typed() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("bns", "alice");

        let cases = [
            ("12345", "user", "did:msgtunnel:12345.user.tg-main"),
            (
                "-100123456",
                "group",
                "did:msgtunnel:-100123456.group.tg-main",
            ),
            (
                "-100987654",
                "channel",
                "did:msgtunnel:-100987654.channel.tg-main",
            ),
        ];

        for (account_id, account_type, expected) in cases {
            let hint = json!({ "account_type": account_type, "tunnel_instance_id": "tg-main" });
            let first = mgr
                .resolve_did(
                    "telegram".to_string(),
                    account_id.to_string(),
                    Some(hint.clone()),
                    Some(owner.clone()),
                )
                .await
                .unwrap();
            // Stable: same triple resolves to the same DID.
            let second = mgr
                .resolve_did(
                    "telegram".to_string(),
                    account_id.to_string(),
                    Some(hint),
                    Some(owner.clone()),
                )
                .await
                .unwrap();
            assert_eq!(first.to_string(), expected);
            assert_eq!(first, second);

            // The typed binding fields are populated.
            let contact = mgr
                .get_contact(first.clone(), Some(owner.clone()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(contact.bindings[0].account_type, account_type);
            assert_eq!(contact.bindings[0].endpoint_did.as_ref(), Some(&first));
            assert_eq!(contact.bindings[0].tunnel_instance_id, "tg-main");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_target_requires_a_unique_endpoint_binding() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("bns", "alice");
        let contact_did = DID::new("bns", "bob");

        // Bind one telegram endpoint and one email endpoint to bob.
        let tg_endpoint = mgr
            .resolve_endpoint_did(
                "telegram".to_string(),
                "12345".to_string(),
                "user".to_string(),
                "tg-main".to_string(),
                Some(owner.clone()),
            )
            .await
            .unwrap();
        assert_eq!(tg_endpoint.to_string(), "did:msgtunnel:12345.user.tg-main");

        mgr.upsert_zone_user_contacts(
            vec![ZoneUserContactSeed {
                did: contact_did.clone(),
                name: "Bob".to_string(),
                note: None,
                bindings: vec![
                    AccountBinding {
                        platform: "telegram".to_string(),
                        account_id: "12345".to_string(),
                        display_id: "@bob".to_string(),
                        tunnel_instance_id: "tg-main".to_string(),
                        account_type: "user".to_string(),
                        endpoint_did: Some(tg_endpoint.clone()),
                        last_active_at: 1,
                        meta: HashMap::new(),
                    },
                    AccountBinding {
                        platform: "email".to_string(),
                        account_id: "bob@example.com".to_string(),
                        display_id: "bob@example.com".to_string(),
                        tunnel_instance_id: "email-main".to_string(),
                        account_type: "addr".to_string(),
                        endpoint_did: Some(DID::new(
                            "msgtunnel",
                            "bob%40example.com.addr.email-main",
                        )),
                        last_active_at: 1,
                        meta: HashMap::new(),
                    },
                ],
                groups: vec!["zone_user".to_string()],
                tags: vec![],
            }],
            Some(owner.clone()),
        )
        .await
        .unwrap();

        // Selector by tunnel_instance_id resolves to the right endpoint DID.
        let resolved = mgr
            .resolve_target(
                contact_did.clone(),
                "tg-main".to_string(),
                Some(owner.clone()),
            )
            .await
            .unwrap();
        assert_eq!(resolved, tg_endpoint);

        // Reverse lookup finds the owning contact.
        let owner_contact = mgr
            .resolve_contact_for_endpoint(tg_endpoint.clone(), Some(owner.clone()))
            .await
            .unwrap();
        assert_eq!(owner_contact, Some(contact_did.clone()));

        // An unknown selector fails — no fallback.
        assert!(mgr
            .resolve_target(
                contact_did.clone(),
                "signal".to_string(),
                Some(owner.clone())
            )
            .await
            .is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn merge_keeps_source_as_resolvable_alias() {
        let (mgr, _tmp) = new_test_mgr().await;

        let target = mgr
            .resolve_did(
                "telegram".to_string(),
                "keep-target".to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        let source = mgr
            .resolve_did(
                "email".to_string(),
                "keep-source@example.com".to_string(),
                None,
                None,
            )
            .await
            .unwrap();

        mgr.merge_contacts(target.clone(), source.clone(), None)
            .await
            .unwrap();

        // The merged-away source DID still resolves to the canonical target.
        let canonical = mgr
            .resolve_canonical_did(source.clone(), None)
            .await
            .unwrap();
        assert_eq!(canonical, target);

        // And it is listed as an alias of the target. A non-merged DID is unchanged.
        let aliases = mgr.list_alias_dids(target.clone(), None).await.unwrap();
        assert!(aliases.contains(&source));
        assert_eq!(
            mgr.resolve_canonical_did(target.clone(), None)
                .await
                .unwrap(),
            target
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn group_subscribers_support_paging() {
        let (mgr, _tmp) = new_test_mgr().await;
        let group_id = DID::new("bns", "group-1");

        mgr.set_group_subscribers(
            group_id.clone(),
            vec![
                DID::new("bns", "u1"),
                DID::new("bns", "u2"),
                DID::new("bns", "u2"),
                DID::new("bns", "u3"),
            ],
            None,
        )
        .await
        .unwrap();

        let page = mgr
            .get_group_subscribers(group_id, Some(2), Some(1), None)
            .await
            .unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0], DID::new("bns", "u2"));
        assert_eq!(page[1], DID::new("bns", "u3"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_zone_users_creates_friend_contact() {
        let (mgr, _tmp) = new_test_mgr().await;
        let owner = DID::new("web", "jarvis.test.buckyos.io");
        let zone_user_did = DID::new("bns", "alice");

        let updated = mgr
            .upsert_zone_user_contacts(
                vec![ZoneUserContactSeed {
                    did: zone_user_did.clone(),
                    name: "Alice".to_string(),
                    note: Some("zone profile".to_string()),
                    bindings: vec![binding("telegram", "user:10001")],
                    groups: vec!["ops".to_string()],
                    tags: vec!["internal".to_string()],
                }],
                Some(owner.clone()),
            )
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let contact = mgr
            .get_contact(zone_user_did.clone(), Some(owner.clone()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(contact.name, "Alice");
        assert_eq!(contact.source, ContactSource::Shared);
        assert_eq!(contact.access_level, AccessGroupLevel::Friend);
        assert!(contact
            .groups
            .iter()
            .any(|group| group.eq_ignore_ascii_case("zone_user")));

        let resolved = mgr
            .resolve_did(
                "telegram".to_string(),
                "user:10001".to_string(),
                None,
                Some(owner),
            )
            .await
            .unwrap();
        assert_eq!(resolved, zone_user_did);
    }
}
