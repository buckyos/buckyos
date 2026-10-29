use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use buckyos_api::{
    bucket_client::BucketId, init_buckyos_api_runtime, parse_bucket_obj_url, BuckyOSRuntimeType,
    NFS_SERVER_SERVICE_NAME,
};
use libopendan::session_context::{ServiceIdentity, TokenKey};
use ndn_lib::{load_named_obj, FileObject};
use opendan::objects::{put_artifact, ARTIFACTS_BUCKET};
use serde_json::json;
use tokio::io::AsyncReadExt;

#[tokio::test]
#[ignore = "requires a running Agent container and OPENDAN_DV_AGENT_ROOT/OPENDAN_DV_SESSION_ID or OPENDAN_DV_SESSION_TOKEN_FILE"]
async fn container_artifact_round_trip_through_node_gateway() -> Result<()> {
    let runtime = init_buckyos_api_runtime("", None, BuckyOSRuntimeType::AppService).await?;
    let app_id = runtime.get_app_id();
    let owner = runtime
        .get_owner_user_id()
        .context("runtime owner is missing")?;
    let gateway = format!(
        "http://{}:{}",
        runtime.get_local_gateway_host(),
        runtime.node_gateway_port
    );
    let token = match std::env::var("OPENDAN_DV_SESSION_TOKEN_FILE") {
        Ok(path) => std::fs::read_to_string(path)?.trim().to_owned(),
        Err(_) => {
            let root = PathBuf::from(std::env::var("OPENDAN_DV_AGENT_ROOT")?);
            let sid = std::env::var("OPENDAN_DV_SESSION_ID")?;
            ensure!(
                root.join("state/session_token.key").is_file(),
                "existing Agent session key required"
            );
            let key = TokenKey::load_or_create(&root)?;
            let helper_token = key.sign(&app_id, &sid, libopendan::now_ms() + 60_000);
            let helper = kRPC::kRPC::new_with_timeout_secs(
                &format!("{gateway}/kapi/{app_id}"),
                Some(helper_token),
                30,
            );
            let identity: ServiceIdentity = serde_json::from_value(
                helper
                    .call("session.service_token", json!({"sid": sid}))
                    .await?,
            )?;
            ensure!(
                identity.app_id == app_id && identity.owner_user_id == owner,
                "Agent identity mismatch"
            );
            identity.token
        }
    };
    ensure!(!token.is_empty(), "session token is empty");
    *runtime.session_token.write().await = token;
    let endpoint = runtime
        .get_zone_service_url(NFS_SERVER_SERVICE_NAME, false)
        .await?;
    ensure!(
        endpoint == format!("{gateway}/kapi/{NFS_SERVER_SERVICE_NAME}"),
        "bucket service bypasses node gateway: {endpoint}"
    );

    let temporary = tempfile::Builder::new()
        .prefix("container-artifact-dv-")
        .tempdir()?;
    let name = format!(
        "{}.md",
        temporary.path().file_name().unwrap().to_string_lossy()
    );
    let bytes = format!("# Container artifact DV\n\n{name}\n").into_bytes();
    let path = temporary.path().join(&name);
    std::fs::write(&path, &bytes)?;
    let bucket = BucketId::user(&app_id, &owner, ARTIFACTS_BUCKET)?;
    let client = runtime.get_bucket_client().await?;
    let store = runtime.get_named_store().await?;
    let stored = put_artifact(&store, &client, &bucket, &path, &name)
        .await
        .map_err(anyhow::Error::msg)?;
    let (bound_bucket, key) =
        parse_bucket_obj_url(stored.url.as_deref().context("artifact URL is missing")?)
            .context("invalid artifact URL")?;
    let verified: Result<()> = async {
        ensure!(
            bound_bucket == bucket,
            "artifact was bound in another bucket"
        );
        let binding = client.get(&bucket, &key, None).await?;
        ensure!(
            binding.obj_id.as_deref() == Some(stored.obj_id.to_string().as_str()),
            "bucket object mismatch"
        );
        let file: FileObject = load_named_obj(&store.get_object(&stored.obj_id).await?)?;
        ensure!(file.content_obj.name == name, "artifact name changed");
        let (mut reader, size) = store.open_reader(&stored.obj_id, None).await?;
        let mut downloaded = Vec::new();
        reader.read_to_end(&mut downloaded).await?;
        ensure!(
            size == bytes.len() as u64 && downloaded == bytes,
            "artifact content mismatch"
        );
        println!(
            "Verified {endpoint}: {bucket}/{key} ({} bytes)",
            downloaded.len()
        );
        Ok(())
    }
    .await;
    let cleanup = client.delete(&bucket, &key, None).await;
    verified?;
    ensure!(cleanup?.deleted, "test artifact binding was not deleted");
    println!("Deleted test artifact binding {bucket}/{key}");
    Ok(())
}
