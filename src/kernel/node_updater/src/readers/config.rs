use super::*;
use buckyos_api::*;
use serde::de::DeserializeOwned;
use serde_json::Value;
use system_config::sled_provider::SledStore;

#[path = "../../../../frame/smb_service/src/samba.rs"]
mod samba;

const STAGED: &str = "system-config";

pub(super) fn open(ctx: &mut Context) -> Result<(), Failure> {
    let path = ctx.root.join("local/system_config");
    if !ctx.exists(&path)? {
        return Ok(());
    }
    let copy = ctx.work.join(STAGED);
    copy_tree(&path, &copy).map_err(|e| ctx.error(path.display(), e))?;
    let db = SledStore::open_existing(&copy).map_err(|e| match e {
        sled::Error::Unsupported(_) => ctx.unsupported(path.display(), e),
        _ => ctx.error(path.display(), e),
    })?;
    ctx.snapshot = db.snapshot().map_err(|e| ctx.error(path.display(), e))?;
    for (k, v) in &ctx.snapshot {
        if k.starts_with(b"__meta/revision/") {
            std::str::from_utf8(v)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or_else(|| {
                    ctx.unsupported(String::from_utf8_lossy(k), "invalid revision counter")
                })?;
        }
    }
    Ok(())
}

pub(super) fn decode<T: DeserializeOwned>(ctx: &Context, key: &str, bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|e| ctx.unsupported(key, e))
}

pub(super) fn kernel(ctx: &mut Context) -> Result<(), Failure> {
    for key in [
        BUCKYOS_INFO_KEY,
        BUCKYOS_DEV_CONFIG_KEY,
        ZONE_PRESENTATION_KEY,
        DOMAIN_HOSTING_KEY,
    ] {
        if let Some(v) = ctx.snapshot.get(key.as_bytes()) {
            match key {
                BUCKYOS_INFO_KEY => decode::<BuckyOSInfo>(ctx, key, v)?.validate(),
                BUCKYOS_DEV_CONFIG_KEY => decode::<BuckyOSDevConfig>(ctx, key, v)?.validate(),
                ZONE_PRESENTATION_KEY => decode::<ZonePresentationConfig>(ctx, key, v)?.validate(),
                _ => decode::<DomainHosting>(ctx, key, v)?.validate(),
            }
            .map_err(|e| ctx.unsupported(key, e))?;
        }
    }
    Ok(())
}

pub(super) fn applications(ctx: &mut Context) -> Result<(), Failure> {
    for (k, v) in &ctx.snapshot {
        let Ok(key) = std::str::from_utf8(k) else {
            continue;
        };
        let parts: Vec<_> = key.split('/').collect();
        match parts.as_slice() {
            ["system", "app_registry"] => decode::<AppRegistry>(ctx, key, v)?
                .validate(&Default::default())
                .map_err(|e| ctx.unsupported(key, e))?,
            ["users", owner, "agents", agent, "spec"] => {
                let spec: AgentSpec = decode(ctx, key, v)?;
                spec.validate().map_err(|e| ctx.unsupported(key, e))?;
                if spec.agent_id.as_str() != *agent
                    || !files::component(owner)
                    || !files::component(agent)
                {
                    return Err(ctx.unsupported(key, "invalid agent identity in registered path"));
                }
                ctx.agents.push(
                    ctx.root
                        .join("data/home")
                        .join(owner)
                        .join("agents")
                        .join(agent),
                );
            }
            ["users", _, "apps", _, "install_record"] => {
                let record: InstallRecord = decode(ctx, key, v)?;
                require(ctx, key, record.schema_version, APP_INSTALL_SCHEMA_VERSION)?;
            }
            ["users", _, "agents", _, "install_record"] => {
                let record: AgentInstallRecord = decode(ctx, key, v)?;
                require(ctx, key, record.schema_version, AGENT_SPEC_SCHEMA_VERSION)?;
            }
            _ => continue,
        }
    }
    Ok(())
}

fn require(ctx: &Context, key: &str, actual: u32, target: u32) -> Result<(), Failure> {
    if actual == target {
        Ok(())
    } else {
        Err(ctx.unsupported(key, format!("actual schema {actual}, target {target}")))
    }
}

pub(super) fn settings(ctx: &mut Context) -> Result<(), Failure> {
    for (k, v) in &ctx.snapshot {
        let Ok(key) = std::str::from_utf8(k) else {
            continue;
        };
        let parts: Vec<_> = key.split('/').collect();
        match parts.as_slice() {
            ["services", "aicc", "driver_metadata" | "settings"] => {
                let raw = std::str::from_utf8(v).map_err(|e| ctx.unsupported(key, e))?;
                aicc::validate_persisted_settings(key, raw).map_err(|e| ctx.unsupported(key, e))?;
            }
            ["services", "smb-service", "latest_users"] => {
                let _: Vec<samba::SmbUserItem> = decode(ctx, key, v)?;
            }
            ["services", "smb-service", "latest_smb_items"] => {
                let _: Vec<samba::SmbItem> = decode(ctx, key, v)?;
            }
            ["users", _, "samba", "settings"] => {
                let _: samba::UserSambaInfo = decode(ctx, key, v)?;
            }
            ["users", _, "apps", "text-editor.buckyos.bns.did", "settings"] => {
                let d: Value = decode(ctx, key, v)?;
                if d["schema_version"] != 1 {
                    return Err(ctx.unsupported(key, "Text Editor requires schema_version 1"));
                }
            }
            _ => (),
        }
    }
    Ok(())
}
