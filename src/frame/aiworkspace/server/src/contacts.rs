use aiworkspace_core::{wish::record_id_for_key, Code, WsError, WsResult};
use aiworkspace_store::source::Fetched;
use async_trait::async_trait;
use buckyos_api::{
    contact_tag_kind, get_buckyos_api_runtime, obj_service_url, Contact, ContactQuery,
    ContactTagKind,
};
use name_lib::DID;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, Write};

const PAGE_SIZE: usize = 200;

#[async_trait]
pub trait ContactsClient: Send + Sync {
    async fn list(&self, owner: &DID, offset: u64, limit: usize) -> WsResult<Vec<Contact>>;
    async fn get(&self, owner: &DID, did: &DID) -> WsResult<Option<Contact>>;
}

pub struct RuntimeContacts;

async fn service_client() -> WsResult<buckyos_api::MsgCenterClient> {
    let runtime =
        get_buckyos_api_runtime().map_err(|_| upstream("Message Center is unavailable"))?;
    runtime
        .get_msg_center_client()
        .await
        .map_err(|_| upstream("Message Center is unavailable"))
}

fn upstream(detail: &str) -> WsError {
    WsError::new(Code::DependencyUnavailable, detail)
}

fn read_error(error: kRPC::RPCErrors) -> WsError {
    match error {
        kRPC::RPCErrors::NoPermission(_) | kRPC::RPCErrors::InvalidToken(_) => {
            upstream("Message Center refused access to the owner's contacts")
        }
        _ => upstream("Message Center could not read the owner's contacts"),
    }
}

#[async_trait]
impl ContactsClient for RuntimeContacts {
    async fn list(&self, owner: &DID, offset: u64, limit: usize) -> WsResult<Vec<Contact>> {
        let query = ContactQuery {
            offset: Some(offset),
            limit: Some(limit),
            ..Default::default()
        };
        service_client()
            .await?
            .list_contacts(query, Some(owner.clone()))
            .await
            .map_err(read_error)
    }

    async fn get(&self, owner: &DID, did: &DID) -> WsResult<Option<Contact>> {
        service_client()
            .await?
            .get_contact(did.clone(), Some(owner.clone()))
            .await
            .map_err(read_error)
    }
}

struct SizeBudget(usize);

impl Write for SizeBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("source size limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SizeBudget {
    fn charge(&mut self, value: &impl Serialize) -> WsResult<()> {
        serde_json::to_writer(self, value)
            .map_err(|_| WsError::limit("the contacts source exceeds the fetch size limit"))
    }
}

fn project(owner: &str, contact: Contact) -> Value {
    let did = contact.did.to_string();
    let mut tags: Vec<String> = contact
        .tags
        .into_iter()
        .filter(|t| {
            contact_tag_kind(t) == ContactTagKind::User || contact.projected_from.contains(t)
        })
        .collect();
    tags.sort_unstable();
    tags.dedup();
    json!({
        "record_id": record_id_for_key(&[json!(did)]),
        "values": {
            "did": did,
            "name": contact.name,
            "tags": tags,
            "friend_state": contact.friend_state,
            "contact_ref": obj_service_url("msg-center", &["owners", owner, "contacts", &did]),
        }
    })
}

pub async fn fetch(
    client: &dyn ContactsClient,
    owner_did: &str,
    previous_dids: &[String],
    max_bytes: usize,
) -> WsResult<(Fetched, u64)> {
    let owner =
        DID::from_str(owner_did).map_err(|_| WsError::denied("the bound owner DID is invalid"))?;
    let fields: Vec<Value> = [
        ("did", "text"),
        ("name", "text"),
        ("tags", "json"),
        ("friend_state", "text"),
        ("contact_ref", "text"),
    ]
    .into_iter()
    .map(|(id, ty)| json!({ "field_id": id, "name": id, "type": ty }))
    .collect();
    let mut records = BTreeMap::new();
    let mut transferred = SizeBudget(max_bytes);
    let mut projected = SizeBudget(max_bytes);
    projected.charge(&fields)?;
    let mut offset = 0;
    loop {
        let page = client.list(&owner, offset, PAGE_SIZE).await?;
        let len = page.len();
        if len > PAGE_SIZE {
            return Err(upstream(
                "Message Center returned more contacts than requested",
            ));
        }
        transferred.charge(&page)?;
        for contact in page {
            let key = contact.did.to_string();
            let row = project(owner_did, contact);
            projected.charge(&row)?;
            if records.insert(key, row).is_some() {
                return Err(WsError::sub(
                    Code::InvalidSchema,
                    "DUPLICATE_KEY",
                    "a contact DID appeared twice while paging the address book",
                ));
            }
        }
        if len < PAGE_SIZE {
            break;
        }
        offset += len as u64;
    }
    for key in previous_dids {
        if records.contains_key(key) {
            continue;
        }
        let did = DID::from_str(key).map_err(|_| {
            WsError::invalid_schema("the last contacts version contains an invalid DID")
        })?;
        let contact = client.get(&owner, &did).await?;
        transferred.charge(&contact)?;
        if let Some(contact) = contact {
            if contact.did != did {
                return Err(upstream(
                    "Message Center returned a different DID during a contact recheck",
                ));
            }
            let row = project(owner_did, contact);
            projected.charge(&row)?;
            records.insert(key.clone(), row);
        }
    }
    Ok((
        Fetched::Table {
            fields,
            records: records.into_values().collect(),
        },
        (max_bytes - transferred.0) as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Authenticator, StaticTokens};
    use std::collections::HashMap;
    use std::sync::Mutex;

    const ALICE: &str = "did:bns:alice";
    const BOB: &str = "did:bns:bob";
    const MAX_BYTES: usize = 32 * 1024 * 1024;

    #[derive(Default)]
    struct Books {
        visible: HashMap<String, Vec<Contact>>,
        rechecks: HashMap<String, Contact>,
        checked: Mutex<Vec<(String, String)>>,
        fail_offset: Option<u64>,
        fail_get: bool,
    }

    #[async_trait]
    impl ContactsClient for Books {
        async fn list(&self, owner: &DID, offset: u64, limit: usize) -> WsResult<Vec<Contact>> {
            if self.fail_offset == Some(offset) {
                return Err(upstream("page unavailable"));
            }
            let records = self
                .visible
                .get(&owner.to_string())
                .ok_or_else(|| upstream("unknown owner"))?;
            Ok(records
                .iter()
                .skip(offset as usize)
                .take(limit)
                .cloned()
                .collect())
        }

        async fn get(&self, owner: &DID, did: &DID) -> WsResult<Option<Contact>> {
            if self.fail_get {
                return Err(upstream("single contact unavailable"));
            }
            self.checked
                .lock()
                .unwrap()
                .push((owner.to_string(), did.to_string()));
            Ok(self.rechecks.get(&did.to_string()).cloned())
        }
    }

    fn contact(i: usize) -> Contact {
        serde_json::from_value(json!({
            "did": format!("did:web:p{i}.example"), "name": "same name", "source": "shared", "is_verified": true,
            "access_level":"allow", "friend_state":"outgoing_pending", "created_at":1, "updated_at":2,
            "tags":["zone_user", "vip", "homestation.following", "group:other", "group:team", "vip"],
            "projected_from":["group:team"], "note":"private note", "avatar":"private avatar",
            "bindings":[{"platform":"private-platform", "account_id":"private-account", "display_id":"private-display", "tunnel_instance_id":"private-endpoint", "last_active_at":3}]
        })).unwrap()
    }

    #[tokio::test]
    async fn pages_the_complete_bound_book_and_rechecks_only_missing_dids() {
        let mut books = Books::default();
        books
            .visible
            .insert(ALICE.into(), (0..1001).map(contact).collect());
        books.visible.insert(BOB.into(), vec![]);
        let mut renamed = contact(2001);
        renamed.name = "renamed".into();
        let renamed_did = renamed.did.to_string();
        let removed = contact(2002).did.to_string();
        books.rechecks.insert(renamed_did.clone(), renamed);
        let before = [
            contact(1).did.to_string(),
            renamed_did.clone(),
            removed.clone(),
        ];
        let (Fetched::Table { fields, records }, _) =
            fetch(&books, ALICE, &before, MAX_BYTES).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(records.len(), 1002);
        let row = records
            .iter()
            .find(|r| r["values"]["did"] == renamed_did)
            .unwrap();
        let wanted = json!({
            "did":renamed_did, "name":"renamed", "tags":["group:team", "vip"], "friend_state":"outgoing_pending",
            "contact_ref":obj_service_url("msg-center", &["owners", ALICE, "contacts", &renamed_did])
        });
        assert_eq!(row["values"], wanted);
        assert_eq!(row["record_id"], record_id_for_key(&[json!(renamed_did)]));
        assert_eq!(
            *books.checked.lock().unwrap(),
            vec![
                (ALICE.to_string(), renamed_did),
                (ALICE.to_string(), removed)
            ]
        );
        let (
            Fetched::Table {
                fields: empty_fields,
                records,
            },
            _,
        ) = fetch(&books, BOB, &[], MAX_BYTES).await.unwrap()
        else {
            panic!()
        };
        assert!(records.is_empty());
        assert_eq!(fields, empty_fields);
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[2]["type"], "json");
    }

    #[tokio::test]
    async fn discards_partial_candidates_on_failure_overlap_or_limit() {
        let mut books = Books::default();
        let mut contacts: Vec<_> = (0..200).map(contact).collect();
        contacts.push(contact(1));
        books.visible.insert(ALICE.into(), contacts);
        books.fail_offset = Some(200);
        assert_eq!(
            fetch(&books, ALICE, &[], MAX_BYTES).await.unwrap_err().code,
            Code::DependencyUnavailable
        );
        books.fail_offset = None;
        assert_eq!(
            fetch(&books, ALICE, &[], MAX_BYTES).await.unwrap_err().sub,
            Some("DUPLICATE_KEY")
        );
        books.visible.get_mut(ALICE).unwrap().pop();
        books.fail_get = true;
        assert_eq!(
            fetch(&books, ALICE, &[contact(201).did.to_string()], MAX_BYTES)
                .await
                .unwrap_err()
                .code,
            Code::DependencyUnavailable
        );
        assert_eq!(
            fetch(&books, ALICE, &[], 10_000).await.unwrap_err().code,
            Code::LimitExceeded
        );
    }

    #[tokio::test]
    async fn identity_is_read_again_when_the_account_or_did_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mut auth = StaticTokens::default();
        auth.source = Some(path.clone());
        let update = |records: Value| {
            std::fs::write(&path, json!({ "tokens": records }).to_string()).unwrap()
        };
        update(
            json!({"a":{"class":"account","principal":"alice","did":ALICE},"b":{"principal":"bob","did":BOB}}),
        );
        assert_eq!(auth.contacts_owner_did("alice").await.unwrap(), ALICE);
        assert_eq!(auth.contacts_owner_did("bob").await.unwrap(), BOB);
        update(json!({"a":{"principal":"alice","did":"did:bns:replacement"}}));
        assert_eq!(
            auth.contacts_owner_did("alice").await.unwrap(),
            "did:bns:replacement"
        );
        assert_eq!(
            auth.contacts_owner_did("bob").await.unwrap_err().code,
            Code::PermissionDenied
        );
        update(json!({"a":{"principal":"alice","did":ALICE,"restricted":true}}));
        assert_eq!(
            auth.contacts_owner_did("alice").await.unwrap_err().code,
            Code::PermissionDenied
        );
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            auth.contacts_owner_did("alice").await.unwrap_err().code,
            Code::DependencyUnavailable
        );
    }
}
