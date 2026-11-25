use super::{files, Context, Failure};
use buckyos_api::{
    build_current_rbac_config, decode_device_config_without_verify, oauth_subject_key,
    user_did_from_doc_value, OAuthSubject, RetiredOAuthSource, SsoSettings, UserSettings, UserType,
};
use name_lib::{DeviceInfo, UserPrivateProfile, DID};

pub(super) fn identity(ctx: &mut Context) -> Result<(), Failure> {
    for (raw_key, bytes) in &ctx.snapshot {
        let Ok(key) = std::str::from_utf8(raw_key) else {
            continue;
        };
        let parts: Vec<_> = key.split('/').collect();
        let text = || std::str::from_utf8(bytes).map_err(|e| ctx.unsupported(key, e));
        match parts.as_slice() {
            ["system", "rbac", "policy"] => {
                let tail = text()?;
                for (i, line) in tail.lines().map(str::trim).enumerate() {
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let fields: Vec<_> = line.split(',').map(str::trim).collect();
                    if !matches!(
                        fields.as_slice(),
                        ["p", _, _, _, "allow" | "deny"] | ["g", _, _]
                    ) {
                        return Err(ctx
                            .unsupported(key, format!("unknown policy layout at line {}", i + 1)));
                    }
                }
                let rule = build_current_rbac_config(Some(tail));
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| ctx.error(key, e))?
                    .block_on(rbac::create_enforcer(&rule.model, &rule.policy))
                    .map_err(|_| ctx.unsupported(key, "cannot load stored RBAC policy"))?;
            }
            ["users", _, "doc"] => {
                user_did_from_doc_value(key, text()?).map_err(|e| ctx.unsupported(key, e))?;
            }
            ["users", owner, "settings"] => {
                let settings: UserSettings = serde_json::from_slice(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized user settings"))?;
                // Scheduler stores the zone root account at users/root/settings
                // under the owner's user ID (SystemConfigBuilder::add_default_accounts).
                if *owner == "root" {
                    if !matches!(settings.user_type, UserType::Root) {
                        return Err(ctx.unsupported(key, "root settings must have type root"));
                    }
                } else if settings.user_id != *owner {
                    return Err(ctx.unsupported(key, "user ID differs from the config path"));
                }
            }
            ["users", _, "profile"] => {
                serde_json::from_slice::<UserPrivateProfile>(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized private profile"))?;
            }
            ["devices", name, "doc"] => {
                let doc = decode_device_config_without_verify(text()?)
                    .map_err(|_| ctx.unsupported(key, "unrecognized signed device document"))?;
                if doc.name != *name {
                    return Err(ctx.unsupported(key, "device name differs from the config path"));
                }
            }
            ["devices", name, "info"] => {
                let info: DeviceInfo = serde_json::from_slice(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized device info"))?;
                if info.device_doc.name != *name {
                    return Err(ctx.unsupported(key, "device name differs from the config path"));
                }
            }
            _ => continue,
        }
    }
    Ok(())
}

pub(super) fn sso(ctx: &mut Context) -> Result<(), Failure> {
    for (raw_key, bytes) in &ctx.snapshot {
        let Ok(key) = std::str::from_utf8(raw_key) else {
            continue;
        };
        let parts: Vec<_> = key.split('/').collect();
        let account = |name: &str| {
            if files::component(name) {
                Ok(())
            } else {
                Err(ctx.unsupported(key, "unrecognized account binding"))
            }
        };
        match parts.as_slice() {
            ["system", "sso", "settings"] => {
                serde_json::from_slice::<SsoSettings>(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized SSO settings"))?;
            }
            ["services", "control_panel", "oauth", "retired_sources"] => {
                serde_json::from_slice::<Vec<RetiredOAuthSource>>(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized retired login sources"))?;
            }
            ["services", "control_panel", "oauth", "subjects", id] => {
                let subject: OAuthSubject = serde_json::from_slice(bytes)
                    .map_err(|_| ctx.unsupported(key, "unrecognized OAuth subject"))?;
                if oauth_subject_key(&subject.identity.issuer, &subject.identity.sub) != *id {
                    return Err(ctx.unsupported(key, "OAuth subject differs from the config path"));
                }
            }
            ["services", "control_panel", "oauth", "labels", _] => {
                let hash = |c: &u8| matches!(c, b'0'..=b'9' | b'a'..=b'f');
                if bytes.len() != 64 || !bytes.iter().all(hash) {
                    return Err(ctx.unsupported(key, "unrecognized OAuth identity index"));
                }
            }
            ["services", "control_panel", "oauth", "bindings", _] => {
                account(std::str::from_utf8(bytes).unwrap_or_default())?;
            }
            ["services", "control_panel", "native_login_bindings", _] => {
                account(&serde_json::from_slice::<String>(bytes).unwrap_or_default())?;
            }
            ["services", "control_panel", "user_login_sources", _, _] => {
                if !bytes.starts_with(b"\"did:") || serde_json::from_slice::<DID>(bytes).is_err() {
                    return Err(ctx.unsupported(key, "unrecognized login source"));
                }
            }
            _ => continue,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifying_identity_records_preserves_authorization_text_and_revisions() {
        let root = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = root.path().join("local/system_config");
        let policy = b"# local grant\r\ng, demo, users\r\n";
        {
            drop(system_config::sled_provider::SledStore::open(&path).unwrap());
            let db = sled::open(&path).unwrap();
            db.insert(b"system/rbac/policy", policy.as_slice()).unwrap();
            db.insert(b"users/demo/doc", br#"{"id":"did:bns:demo"}"#.as_slice())
                .unwrap();
            db.flush().unwrap();
        }
        let run = super::super::prepare(
            root.path(),
            &root.path().join("security"),
            work.path(),
            &|_| Ok(false),
        )
        .unwrap();
        assert!(run.changes.is_empty());
        let db = sled::open(path).unwrap();
        assert_eq!(
            db.get("system/rbac/policy").unwrap().unwrap().as_ref(),
            policy
        );
        assert!(db.scan_prefix("__meta/revision/").next().is_none());
    }

    #[test]
    fn root_settings_carry_the_owner_id_and_other_accounts_match_their_path() {
        let settings = |user_id: &str, user_type: &str| {
            serde_json::json!({
                "user_id": user_id, "type": user_type, "password": "hash",
                "state": "active", "res_pool_id": "default", "is_local": true,
            })
            .to_string()
        };
        for (key, value, accepted) in [
            // As written by SystemConfigBuilder::add_default_accounts.
            ("users/root/settings", settings("alice", "root"), true),
            ("users/alice/settings", settings("alice", "admin"), true),
            ("users/root/settings", settings("alice", "admin"), false),
            ("users/bob/settings", settings("alice", "user"), false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("local/system_config");
            drop(system_config::sled_provider::SledStore::open(&path).unwrap());
            let db = sled::open(&path).unwrap();
            db.insert(key, value.as_bytes()).unwrap();
            db.flush().unwrap();
            drop(db);
            let result = super::super::prepare(
                root.path(),
                &root.path().join("security"),
                tempfile::tempdir().unwrap().path(),
                &|_| Ok(false),
            );
            match result {
                Ok(run) => assert!(accepted && run.changes.is_empty(), "{key} {value}"),
                Err(e) => assert!(
                    !accepted && e.unsupported && e.module == "identity-config" && e.area == key,
                    "{key} {value}: {e}"
                ),
            }
        }
    }
}
