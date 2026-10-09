//! The zone-level service (§4.4, §4.6): one process serves every user of the zone, like Message
//! Center. Each user gets an own `Station` (stream, inbox, reading pipeline, private state in
//! an own database), opened on first use; the zone feed list aggregates posts users chose to
//! list there; portal methods let anyone — signed in or not — read a user's or the zone's feed.

use crate::audience::Reader;
use crate::auth::Caller;
use crate::directory::Directory;
use crate::error::{bad, HsError, HsResult};
use crate::protocol::{normalize_obj_id, valid_user_segment, EntryNamespace, HomeRef, ZONE_FEED};
use crate::publish::{get_entry, get_head, EntryKind};
use crate::stream::{ChangesPage, DisplayPage};
use crate::users::{UserRegistry, ZoneUser};
use crate::zone::ZoneFeed;
use crate::Station;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub zone: String,
    pub zone_did: Option<String>,
    /// Display name of the zone feed.
    pub zone_name: String,
    /// What `www.<zone>/` and `homestation.<zone>/` open: `<user>` or `~zone`; none = the
    /// zone owner's feed.
    pub default_feed: Option<String>,
    /// The zone owner's username.
    pub owner_user: Option<String>,
    /// Usernames that may list posts in the zone feed (`*` = everyone); none = regular users.
    pub zone_feed_writers: Option<Vec<String>>,
    pub start_workers: bool,
}

impl HostConfig {
    pub fn new(zone: &str) -> Self {
        Self {
            zone: zone.trim_end_matches('.').to_ascii_lowercase(),
            zone_did: None,
            zone_name: zone.to_string(),
            default_feed: None,
            owner_user: None,
            zone_feed_writers: None,
            start_workers: true,
        }
    }
}

/// Builds the station of one user (database under `users/<user>/`, contacts of that user).
pub type StationBuilder = Box<dyn Fn(&ZoneUser) -> HsResult<Arc<Station>> + Send + Sync>;

/// A reader of the zone feed or a portal: anonymous or an authenticated DID. Each station
/// turns it into its own `Reader` (the publisher reading itself is that station's owner).
#[derive(Debug, Clone, PartialEq)]
pub enum Viewer {
    Anonymous,
    Did(String),
}

impl Viewer {
    pub fn from_reader(reader: &Reader, owner: &str) -> Self {
        match reader {
            Reader::Anonymous => Self::Anonymous,
            Reader::Owner => Self::Did(owner.to_string()),
            Reader::Did(d) => Self::Did(d.clone()),
        }
    }

    pub fn did(&self) -> Option<&str> {
        match self {
            Self::Anonymous => None,
            Self::Did(d) => Some(d),
        }
    }
}

impl Station {
    pub fn reader_for(&self, viewer: &Viewer) -> Reader {
        match viewer {
            Viewer::Anonymous => Reader::Anonymous,
            Viewer::Did(d) if *d == self.cfg.owner => Reader::Owner,
            Viewer::Did(d) => Reader::Did(d.clone()),
        }
    }

    /// Tell the zone feed that a listed entry changed (no-op for unlisted entries).
    pub(crate) async fn zone_touch(&self, entry: &str, kind: &str) {
        if let Some(host) = self.host() {
            if let Err(e) = host.zone_feed.touch(entry, kind).await {
                log::warn!("zone feed change for {entry}: {e}");
            }
        }
    }

    pub async fn check_zone_writer(&self) -> HsResult<()> {
        let host = self.host().ok_or_else(|| HsError::Unavailable("no zone feed here".into()))?;
        if host.zone_feed_writer(&self.cfg.user).await {
            Ok(())
        } else {
            Err(HsError::Forbidden("homestation.zoneFeed.notWriter".into()))
        }
    }

    /// List one of my public posts in the zone feed, or take it out again (§4.6).
    pub async fn set_zone_listing(&self, entry: &str, listed: bool) -> HsResult<bool> {
        let host = self.host().ok_or_else(|| HsError::Unavailable("no zone feed here".into()))?;
        let entry_s = entry.to_string();
        let (row, head, iat) = self
            .db
            .call(move |c| {
                let row = get_entry(c, &entry_s)?;
                let head = get_head(c, &entry_s)?;
                let iat: Option<i64> = match head.as_ref().and_then(|h| h.current.clone()) {
                    Some(current) => rusqlite::OptionalExtension::optional(c.query_row("SELECT iat FROM feed_index WHERE obj_id=?1", [current], |r| r.get(0)))?,
                    None => None,
                };
                Ok((row, head, iat))
            })
            .await?;
        let row = row.ok_or_else(|| HsError::NotFound("no such entry".into()))?;
        if listed {
            self.check_zone_writer().await?;
            if !matches!(row.kind, EntryKind::Post | EntryKind::Quote) {
                return Err(bad("homestation.zoneFeed.postsOnly"));
            }
            if !row.audience.is_public() {
                return Err(bad("homestation.validation.zoneFeedPublic"));
            }
            if head.as_ref().map(|h| h.state) != Some(crate::protocol::HeadState::Active) {
                return Err(bad("homestation.zoneFeed.withdrawn"));
            }
        }
        let iat = iat.unwrap_or(row.created_at / 1000);
        let changed = host.zone_feed.set_listed(&self.cfg.user, &self.cfg.owner, entry, iat, listed).await?;
        if changed {
            self.bump(&["published"]);
        }
        Ok(changed)
    }
}

pub struct Host {
    pub cfg: HostConfig,
    pub directory: Arc<dyn Directory>,
    pub auth: Arc<dyn crate::auth::Authenticator>,
    pub users: Arc<UserRegistry>,
    pub zone_feed: ZoneFeed,
    builder: StationBuilder,
    stations: std::sync::Mutex<HashMap<String, Arc<Station>>>,
    me: Weak<Host>,
}

impl Host {
    pub fn new(
        cfg: HostConfig,
        directory: Arc<dyn Directory>,
        auth: Arc<dyn crate::auth::Authenticator>,
        users: Arc<UserRegistry>,
        zone_feed: ZoneFeed,
        builder: StationBuilder,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self { cfg, directory, auth, users, zone_feed, builder, stations: Default::default(), me: me.clone() })
    }

    pub fn home(&self, user: &str) -> HomeRef {
        HomeRef::new(&self.cfg.zone, user)
    }

    /// The station of a zone user, opened on first use; `None` for unknown users.
    pub async fn station(&self, user: &str) -> HsResult<Option<Arc<Station>>> {
        if !valid_user_segment(user) || user == ZONE_FEED {
            return Ok(None);
        }
        if let Some(station) = self.stations.lock().unwrap().get(user) {
            return Ok(Some(station.clone()));
        }
        match self.users.by_user(user).await {
            Some(zone_user) => self.open(&zone_user).map(Some),
            None => Ok(None),
        }
    }

    pub async fn station_for_did(&self, did: &str) -> HsResult<Option<Arc<Station>>> {
        match self.users.by_did(did).await {
            Some(user) => self.station(&user.user).await,
            None => Ok(None),
        }
    }

    /// The caller's own station: session DID first, the session subject (username) otherwise.
    pub async fn station_for_caller(&self, caller: &Caller) -> HsResult<Option<Arc<Station>>> {
        if let Some(did) = &caller.did {
            return self.station_for_did(did).await;
        }
        match self.users.by_user(&caller.principal).await {
            Some(user) => self.station(&user.user).await,
            None => Ok(None),
        }
    }

    /// Open (or return) the station of a known user.
    pub fn open(&self, user: &ZoneUser) -> HsResult<Arc<Station>> {
        let mut stations = self.stations.lock().unwrap();
        if let Some(station) = stations.get(&user.user) {
            return Ok(station.clone());
        }
        let station = (self.builder)(user)?;
        station.attach_host(self.me.clone());
        if self.cfg.start_workers {
            station.start_workers();
        }
        log::info!("opened HomeStation of {} ({})", user.user, user.did);
        stations.insert(user.user.clone(), station.clone());
        Ok(station)
    }

    /// Open every user's station: all of them have a stream and an inbox from the start.
    pub async fn open_all(&self) -> HsResult<usize> {
        let users = self.users.list().await;
        for user in &users {
            self.open(user)?;
        }
        Ok(users.len())
    }

    pub fn opened(&self) -> Vec<Arc<Station>> {
        let mut stations: Vec<_> = self.stations.lock().unwrap().values().cloned().collect();
        stations.sort_by(|a, b| a.cfg.user.cmp(&b.cfg.user));
        stations
    }

    pub async fn zone_feed_writer(&self, user: &str) -> bool {
        match &self.cfg.zone_feed_writers {
            Some(list) => list.iter().any(|w| w == "*" || w == user),
            None => self.users.by_user(user).await.is_some_and(|u| u.zone_feed_writer),
        }
    }

    /// `<user>` or `~zone`: what the zone's short domains open at `/`.
    pub async fn default_feed(&self) -> String {
        if let Some(feed) = &self.cfg.default_feed {
            if feed == ZONE_FEED || self.users.by_user(feed).await.is_some() {
                return feed.clone();
            }
            log::warn!("default feed {feed} is not a user of this zone");
        }
        if let Some(owner) = &self.cfg.owner_user {
            return owner.clone();
        }
        self.users.list().await.first().map(|u| u.user.clone()).unwrap_or_else(|| ZONE_FEED.to_string())
    }

    /// `GET /home/`: the zone's homes.
    pub async fn zone_index(&self) -> Value {
        json!({
            "zone": self.cfg.zone,
            "zoneDid": self.cfg.zone_did,
            "zoneName": self.cfg.zone_name,
            "defaultFeed": self.default_feed().await,
            "zoneFeed": { "feed": ZONE_FEED, "stream": self.home(ZONE_FEED).stream() },
        })
    }

    /// `GET /home/?did=`: where this zone keeps the HomeStation of a DID (§4.4 发现).
    pub async fn locate(&self, did: &str) -> Option<Value> {
        let user = self.users.by_did(did).await?;
        let home = self.home(&user.user);
        Some(json!({ "did": user.did, "user": user.user, "stream": home.stream(), "inbox": home.inbox() }))
    }

    /// Display read of the zone feed: listed entries the viewer may see, newest `iat` first.
    pub async fn zone_display(&self, viewer: &Viewer, cursor: Option<String>, limit: usize, with_objects: bool) -> HsResult<DisplayPage> {
        let limit = limit.clamp(1, 100);
        let mut after = cursor.as_deref().and_then(|c| c.split_once('|')).map(|(t, e)| (t.parse::<i64>().unwrap_or(i64::MAX), e.to_string()));
        let mut items = Vec::new();
        let mut objects = BTreeMap::new();
        let mut next = None;
        'pages: loop {
            let rows = self.zone_feed.page(after.clone(), limit * 2).await?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                after = Some((row.iat, row.entry.clone()));
                let Some(station) = self.station(&row.user).await? else { continue };
                let reader = station.reader_for(viewer);
                let Some((item, attached)) = station.zone_item(&reader, &row.entry, with_objects).await? else { continue };
                if items.len() == limit {
                    next = items.last().map(|i: &crate::stream::StreamItem| format!("{}|{}", i.iat, i.entry));
                    break 'pages;
                }
                items.push(item);
                objects.extend(attached);
            }
        }
        Ok(DisplayPage { items, objects, next, change_cursor: self.zone_feed.max_cursor().await? })
    }

    /// Change read of the zone feed: listings, unlistings and the Heads of listed entries.
    pub async fn zone_changes(&self, viewer: &Viewer, since: i64, limit: usize, with_objects: bool) -> HsResult<ChangesPage> {
        let limit = limit.clamp(1, 500);
        let rows = self.zone_feed.changes(since, limit + 1).await?;
        let more = rows.len() > limit;
        let mut changes = Vec::new();
        let mut objects = BTreeMap::new();
        let mut next_cursor = since;
        for row in rows.into_iter().take(limit) {
            next_cursor = row.cursor;
            let Some(station) = self.station(&row.user).await? else { continue };
            let reader = station.reader_for(viewer);
            if let Some((change, attached)) = station.zone_change(&reader, &row.entry, row.cursor, &row.kind, with_objects).await? {
                changes.push(change);
                objects.extend(attached);
            }
        }
        Ok(ChangesPage { changes, objects, next_cursor, more, resync: false })
    }

    pub async fn zone_profile(&self) -> HsResult<Value> {
        let home = self.home(ZONE_FEED);
        Ok(json!({
            "did": self.cfg.zone_did,
            "user": ZONE_FEED,
            "kind": "zone",
            "name": self.cfg.zone_name,
            "bio": "",
            "stream": home.stream(),
            "posts": self.zone_feed.count().await?,
            "featured": [],
        }))
    }

    /// Object of a listed entry, from the publisher's station that grants it to the viewer
    /// (`/home/~zone/objects/<id>`: media of the zone feed portal).
    pub async fn zone_object(&self, viewer: &Viewer, obj_id: &str) -> HsResult<Option<(Arc<Station>, crate::objects::StoredObject)>> {
        let mut users: Vec<String> = Vec::new();
        let mut after = None;
        loop {
            let rows = self.zone_feed.page(after.clone(), 200).await?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                after = Some((row.iat, row.entry.clone()));
                if !users.contains(&row.user) {
                    users.push(row.user.clone());
                }
            }
        }
        for user in users {
            let Some(station) = self.station(&user).await? else { continue };
            let reader = station.reader_for(viewer);
            if let Some(obj) = station.read_object(&reader, obj_id).await? {
                return Ok(Some((station, obj)));
            }
        }
        Ok(None)
    }

    async fn viewer_of(&self, caller: Option<&Caller>) -> (Viewer, Option<ZoneUser>) {
        let Some(caller) = caller else { return (Viewer::Anonymous, None) };
        let user = match &caller.did {
            Some(did) => self.users.by_did(did).await,
            None => self.users.by_user(&caller.principal).await,
        };
        let viewer = caller.did.clone().or_else(|| user.as_ref().map(|u| u.did.clone())).map(Viewer::Did).unwrap_or(Viewer::Anonymous);
        (viewer, user)
    }

    /// kRPC entry: `portal.*` for anyone (session optional), everything else on the caller's
    /// own station.
    pub async fn handle_rpc(&self, caller: Option<&Caller>, method: &str, params: Value) -> HsResult<Value> {
        if let Some(portal) = method.strip_prefix("portal.") {
            return self.portal_rpc(caller, portal, params).await;
        }
        let caller = caller.ok_or_else(|| HsError::Forbidden("unauthorized: session token required".into()))?;
        let station = self
            .station_for_caller(caller)
            .await?
            .ok_or_else(|| HsError::Forbidden("homestation.noHome: this account has no HomeStation in this zone".into()))?;
        match method {
            "zone.set_listing" => {
                let entry: String = serde_json::from_value(params.get("entry").cloned().unwrap_or(Value::Null)).map_err(|e| bad(format!("parameter entry: {e}")))?;
                let listed = params.get("listed").and_then(Value::as_bool).ok_or_else(|| bad("parameter listed"))?;
                station.set_zone_listing(&entry, listed).await?;
                Ok(json!({ "ok": true, "listed": listed }))
            }
            _ => {
                let mut result = station.handle_rpc(caller, method, params.clone()).await?;
                match method {
                    "ui.bootstrap" => result["home"] = self.home_info(&station).await,
                    "published.list" if params.get("owner").and_then(Value::as_str).is_none_or(|o| o == station.cfg.owner) => {
                        self.mark_listed(&mut result).await?;
                    }
                    _ => {}
                }
                Ok(result)
            }
        }
    }

    async fn home_info(&self, station: &Station) -> Value {
        json!({
            "zone": self.cfg.zone,
            "user": station.cfg.user,
            "stream": station.home().stream(),
            "defaultFeed": self.default_feed().await,
            "zoneFeed": { "feed": ZONE_FEED, "name": self.cfg.zone_name, "writer": self.zone_feed_writer(&station.cfg.user).await },
        })
    }

    async fn mark_listed(&self, page: &mut Value) -> HsResult<()> {
        if let Some(entries) = page.get_mut("entries").and_then(Value::as_array_mut) {
            for entry in entries {
                if let Some(url) = entry.get("entry").and_then(Value::as_str).map(str::to_string) {
                    if self.zone_feed.is_listed(&url).await? {
                        entry["zoneFeed"] = json!(true);
                    }
                }
            }
        }
        Ok(())
    }

    async fn feed_station(&self, params: &Value) -> HsResult<Option<Arc<Station>>> {
        let feed = params.get("feed").and_then(Value::as_str).ok_or_else(|| bad("parameter feed"))?;
        if feed == ZONE_FEED {
            return Ok(None);
        }
        self.station(feed).await?.map(Some).ok_or_else(|| HsError::NotFound(format!("homestation.portal.noSuchFeed: {feed}")))
    }

    /// Portal reads (`/homestation/<user>`, `/homestation/~zone`, the short domains' `/`).
    async fn portal_rpc(&self, caller: Option<&Caller>, method: &str, params: Value) -> HsResult<Value> {
        let (viewer, me) = self.viewer_of(caller).await;
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(10).clamp(1, 50) as usize;
        match method {
            "home" => Ok(json!({
                "zone": self.cfg.zone,
                "zoneName": self.cfg.zone_name,
                "defaultFeed": self.default_feed().await,
                "zoneFeed": ZONE_FEED,
                "viewer": match &me {
                    Some(u) => json!({ "did": u.did, "user": u.user, "name": u.name, "zoneFeedWriter": self.zone_feed_writer(&u.user).await }),
                    None => match viewer.did() {
                        Some(did) => json!({ "did": did }),
                        None => Value::Null,
                    },
                },
            })),
            "profile" => match self.feed_station(&params).await? {
                Some(station) => {
                    let reader = station.reader_for(&viewer);
                    let mut profile = station.profile_view(&station.cfg.owner.clone(), reader).await?;
                    profile["user"] = json!(station.cfg.user);
                    profile["kind"] = json!("user");
                    Ok(profile)
                }
                None => {
                    let mut profile = self.zone_profile().await?;
                    profile["hue"] = json!(crate::hue_of(&self.cfg.zone));
                    Ok(profile)
                }
            },
            "list" => match self.feed_station(&params).await? {
                Some(station) => {
                    let reader = station.reader_for(&viewer);
                    let mut p = json!({ "owner": station.cfg.owner, "limit": limit });
                    for key in ["kind", "cursor"] {
                        if let Some(v) = params.get(key).filter(|v| !v.is_null()) {
                            p[key] = v.clone();
                        }
                    }
                    p["reader"] = match &reader {
                        Reader::Owner => json!({ "kind": "owner" }),
                        Reader::Did(d) => json!({ "kind": "did", "did": d }),
                        Reader::Anonymous => json!({ "kind": "anonymous" }),
                    };
                    let mut page = station.published_list(&p).await?;
                    self.mark_listed(&mut page).await?;
                    Ok(page)
                }
                None => self.zone_list(&viewer, params.get("cursor").and_then(Value::as_str).map(str::to_string), limit).await,
            },
            "item" => {
                let station = self.feed_station(&params).await?;
                let key = params.get("key").and_then(Value::as_str);
                let obj_id = params.get("objId").and_then(Value::as_str).map(normalize_obj_id);
                match (station, key) {
                    (Some(station), Some(key)) => {
                        let entry = station.home().entry(EntryNamespace::Feed, key);
                        let reader = station.reader_for(&viewer);
                        Ok(station.entry_card(&reader, &entry).await?.map(|(view, card)| json!({ "entry": view, "card": card })).unwrap_or(Value::Null))
                    }
                    (Some(station), None) => {
                        let obj_id = obj_id.ok_or_else(|| bad("parameter key or objId"))?;
                        let reader = station.reader_for(&viewer);
                        Ok(json!({ "card": station.card(&obj_id, reader).await? }))
                    }
                    (None, _) => {
                        let obj_id = obj_id.ok_or_else(|| bad("parameter objId"))?;
                        match self.zone_object(&viewer, &obj_id).await? {
                            Some((station, _)) => {
                                let reader = station.reader_for(&viewer);
                                Ok(json!({ "card": station.card(&obj_id, reader).await?, "user": station.cfg.user }))
                            }
                            None => Ok(json!({ "card": Value::Null })),
                        }
                    }
                }
            }
            "comments" | "wrapped_body" => {
                let obj_id = params.get("objId").and_then(Value::as_str).map(normalize_obj_id).ok_or_else(|| bad("parameter objId"))?;
                let station = match self.feed_station(&params).await? {
                    Some(station) => station,
                    None => self.zone_object(&viewer, &obj_id).await?.map(|(s, _)| s).ok_or_else(|| HsError::NotFound("no such item".into()))?,
                };
                let reader = station.reader_for(&viewer);
                if !station.object_readable(&reader, &obj_id).await? {
                    return Err(HsError::NotFound("no such item".into()));
                }
                if method == "wrapped_body" {
                    return station.wrapped_body(&obj_id).await;
                }
                let comment_type = params.get("type").and_then(Value::as_str).unwrap_or("text").to_string();
                station.comment_list(&obj_id, "author", &comment_type).await
            }
            _ => Err(HsError::NotFound(format!("unknown method portal.{method}"))),
        }
    }

    /// Zone feed page in the shape of `published.list`: entry views and cards projected by
    /// each publisher's station for this viewer.
    async fn zone_list(&self, viewer: &Viewer, cursor: Option<String>, limit: usize) -> HsResult<Value> {
        let page = self.zone_display(viewer, cursor, limit, false).await?;
        let mut entries = Vec::new();
        let mut cards = Vec::new();
        for item in &page.items {
            let Some(home) = crate::protocol::EntryRef::parse(&item.entry).ok().and_then(|e| e.home().cloned()) else { continue };
            let Some(station) = self.station(&home.user).await? else { continue };
            let reader = station.reader_for(viewer);
            if let Some((mut view, card)) = station.entry_card(&reader, &item.entry).await? {
                view["user"] = json!(home.user);
                view["zoneFeed"] = json!(true);
                entries.push(view);
                if let Some(card) = card {
                    cards.push(card);
                }
            }
        }
        Ok(json!({ "entries": entries, "nextCursor": page.next, "changeCursor": page.change_cursor, "cards": cards }))
    }
}
