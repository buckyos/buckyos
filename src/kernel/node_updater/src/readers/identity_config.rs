use super::{Context, Failure};
use buckyos_api::{
    build_current_rbac_config, decode_device_config_without_verify, user_did_from_doc_value,
    UserSettings,
};
use name_lib::{DeviceInfo, UserPrivateProfile};

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
                if settings.user_id != *owner {
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
}
