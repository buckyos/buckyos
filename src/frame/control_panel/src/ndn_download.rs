//! In-process NDN download for the install pipeline's Acquire stage.
//!
//! TaskMgr 2.0 removed the task-manager built-in download executor: content
//! acquisition belongs to the service that owns the pipeline. This module
//! carries the object-type orchestration that executor used to provide —
//! chunk/file objects pull straight into the named store; AppDoc objects
//! recurse over their sub packages; `pkg`/FileObject metadata wrappers pull
//! their `content` payload then persist the wrapper.

use buckyos_api::{get_buckyos_api_runtime, AppDoc, DownloadTaskOptions};
use log::{info, warn};
use named_store::NamedDataMgr;
use ndn_lib::{cyfs_get_obj_id_from_url, FileObject, ObjId, OBJ_TYPE_PKG};
use ndn_toolkit::cyfs_ndn_client::{CyfsNdnClient, CyfsPullResult};
use serde_json::Value;

pub fn infer_objid_from_url(download_url: &str) -> Option<ObjId> {
    cyfs_get_obj_id_from_url(download_url)
        .ok()
        .map(|(objid, _)| objid)
}

/// Pull `objid` from `download_url` into the zone named store, resolving
/// metadata wrappers and AppDoc sub packages on the way.
///
/// The NDN client futures are not `Send`, so the pull itself runs on a
/// dedicated single-thread LocalSet worker (the same shape the 1.x
/// task-manager download executor used); this wrapper is Send-safe.
pub async fn download_object_to_named_store(
    download_url: &str,
    objid: &ObjId,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsPullResult, String> {
    let job = DownloadJob {
        download_url: download_url.to_string(),
        objid: objid.clone(),
        download_options: download_options.clone(),
    };
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    download_worker_sender()
        .send((job, result_tx))
        .map_err(|_| "download worker unavailable".to_string())?;
    result_rx
        .await
        .map_err(|_| "download worker dropped the job".to_string())?
}

struct DownloadJob {
    download_url: String,
    objid: ObjId,
    download_options: DownloadTaskOptions,
}

type DownloadJobEnvelope = (
    DownloadJob,
    tokio::sync::oneshot::Sender<Result<CyfsPullResult, String>>,
);

fn download_worker_sender() -> &'static tokio::sync::mpsc::UnboundedSender<DownloadJobEnvelope> {
    static SENDER: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<DownloadJobEnvelope>> =
        std::sync::OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<DownloadJobEnvelope>();
        std::thread::Builder::new()
            .name("cp-ndn-download".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("ndn download runtime init must succeed");
                let local_set = tokio::task::LocalSet::new();
                local_set.block_on(&runtime, async move {
                    while let Some((job, result_tx)) = receiver.recv().await {
                        tokio::task::spawn_local(async move {
                            let result = run_download_job(job).await;
                            let _ = result_tx.send(result);
                        });
                    }
                });
            })
            .expect("ndn download worker thread must start");
        sender
    })
}

async fn run_download_job(job: DownloadJob) -> Result<CyfsPullResult, String> {
    let runtime = get_buckyos_api_runtime().map_err(|err| format!("get runtime failed: {err}"))?;
    let named_store = runtime
        .get_named_store()
        .await
        .map_err(|err| format!("get named store failed: {err}"))?;
    let session_token = runtime.get_session_token().await;
    let client = build_ndn_client(
        session_token.as_str(),
        named_store.clone(),
        &job.download_options,
    )?;
    pull_named_store_download(
        &client,
        job.download_url.as_str(),
        &job.objid,
        &named_store,
        &job.download_options,
    )
    .await
}

fn build_ndn_client(
    session_token: &str,
    named_store: NamedDataMgr,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsNdnClient, String> {
    let mut builder = CyfsNdnClient::builder();
    if !session_token.trim().is_empty() {
        builder = builder.session_token(session_token.to_string());
    }
    builder = builder.default_store_mgr(named_store);
    if let Some(default_remote_url) = download_options
        .default_remote_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.default_remote_url(default_remote_url.to_string());
    }
    if let Some(timeout_ms) = download_options.timeout_ms.or_else(|| {
        download_options
            .timeout_secs
            .map(|secs| secs.saturating_mul(1000))
    }) {
        builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    if download_options.obj_id_in_host.unwrap_or(false) {
        builder = builder.obj_id_in_host(true);
    }
    builder.build().map_err(|err| err.to_string())
}

#[derive(Clone)]
struct VerifiedJsonObject {
    obj_id: ObjId,
    obj_json: Value,
    obj_str: String,
}

#[derive(Clone)]
struct SubPkgDownloadSpec {
    key: String,
    download_url: String,
    objid: ObjId,
}

async fn pull_named_store_download(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsPullResult, String> {
    if objid.is_chunk() || objid.is_chunk_list() || objid.is_file_object() {
        return pull_direct_to_named_store(client, download_url, objid, store_mgr).await;
    }

    let verified = fetch_verified_json_object(client, download_url, objid).await?;

    if let Ok(app_doc) = serde_json::from_value::<AppDoc>(verified.obj_json.clone()) {
        return pull_app_doc_to_named_store(
            client,
            download_url,
            verified,
            app_doc,
            store_mgr,
            download_options,
        )
        .await;
    }

    // `pkg` objects are PackageMeta: metadata wrappers whose `content`
    // points at the real payload.
    if objid.obj_type == OBJ_TYPE_PKG {
        let file_obj = serde_json::from_value::<FileObject>(verified.obj_json.clone())
            .map_err(|err| format!("parse pkg object {} as FileObject failed: {}", objid, err))?;
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
        )
        .await;
    }

    if let Ok(file_obj) = serde_json::from_value::<FileObject>(verified.obj_json.clone()) {
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
        )
        .await;
    }

    warn!(
        "download failed to resolve supported object type: objid={} url={}",
        objid, download_url
    );
    Err(format!(
        "unsupported obj type for download: {} ({})",
        objid.obj_type, objid
    ))
}

async fn pull_direct_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
) -> Result<CyfsPullResult, String> {
    info!(
        "direct named object download started: objid={} url={}",
        objid, download_url
    );
    client
        .get(download_url.to_string())
        .obj_id(objid.clone())
        .pull_to_named_store(store_mgr)
        .await
        .map_err(|err| err.to_string())
}

async fn fetch_verified_json_object(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
) -> Result<VerifiedJsonObject, String> {
    let (real_obj_id, obj_str) = client
        .get(download_url.to_string())
        .obj_id(objid.clone())
        .send()
        .await
        .map_err(|err| err.to_string())?
        .object_string()
        .await
        .map_err(|err| err.to_string())?;

    let obj_json = serde_json::from_str::<Value>(obj_str.as_str()).map_err(|err| {
        format!(
            "parse object {} from {} as json failed: {}",
            real_obj_id, download_url, err
        )
    })?;

    Ok(VerifiedJsonObject {
        obj_id: real_obj_id,
        obj_json,
        obj_str,
    })
}

async fn pull_wrapped_file_object_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    verified: VerifiedJsonObject,
    file_obj: FileObject,
    store_mgr: &NamedDataMgr,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsPullResult, String> {
    let content_objid = ObjId::new(file_obj.content.trim()).map_err(|err| {
        format!(
            "invalid wrapped file content obj id for {}: {}",
            verified.obj_id, err
        )
    })?;
    let content_download_url =
        resolve_related_download_url(download_url, &content_objid, download_options)?;

    let mut result = pull_direct_to_named_store(
        client,
        content_download_url.as_str(),
        &content_objid,
        store_mgr,
    )
    .await?;

    store_mgr
        .put_object(&verified.obj_id, verified.obj_str.as_str())
        .await
        .map_err(|err| err.to_string())?;
    push_stored_object(&mut result.stored_objects, verified.obj_id.clone());
    result.obj_id = Some(verified.obj_id);
    if file_obj.size > 0 {
        result.total_size = file_obj.size.max(result.total_size);
    }

    Ok(result)
}

async fn pull_sub_pkg_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsPullResult, String> {
    if objid.is_chunk() || objid.is_chunk_list() || objid.is_file_object() {
        return pull_direct_to_named_store(client, download_url, objid, store_mgr).await;
    }

    let verified = fetch_verified_json_object(client, download_url, objid).await?;

    if serde_json::from_value::<AppDoc>(verified.obj_json.clone()).is_ok() {
        return Err(format!(
            "nested AppDoc sub package is not supported for {}",
            objid
        ));
    }

    if objid.obj_type == OBJ_TYPE_PKG {
        let file_obj = serde_json::from_value::<FileObject>(verified.obj_json.clone())
            .map_err(|err| format!("parse pkg object {} as FileObject failed: {}", objid, err))?;
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
        )
        .await;
    }

    if let Ok(file_obj) = serde_json::from_value::<FileObject>(verified.obj_json.clone()) {
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
        )
        .await;
    }

    Err(format!(
        "unsupported sub package obj type for download: {} ({})",
        objid.obj_type, objid
    ))
}

async fn pull_app_doc_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    verified: VerifiedJsonObject,
    app_doc: AppDoc,
    store_mgr: &NamedDataMgr,
    download_options: &DownloadTaskOptions,
) -> Result<CyfsPullResult, String> {
    store_mgr
        .put_object(&verified.obj_id, verified.obj_str.as_str())
        .await
        .map_err(|err| err.to_string())?;

    let sub_pkgs = resolve_app_doc_sub_pkg_specs(&app_doc, download_url, download_options)?;
    let total_sub_pkgs = sub_pkgs.len();
    info!(
        "AppDoc download resolved: objid={} url={} sub_pkg_count={}",
        verified.obj_id, download_url, total_sub_pkgs
    );

    let mut result = CyfsPullResult {
        obj_id: Some(verified.obj_id.clone()),
        total_size: 0,
        chunk_count: 0,
        stored_objects: vec![verified.obj_id.clone()],
    };

    for (index, sub_pkg) in sub_pkgs.iter().enumerate() {
        info!(
            "AppDoc sub package download: appdoc_objid={} index={}/{} key={} objid={}",
            verified.obj_id,
            index + 1,
            total_sub_pkgs,
            sub_pkg.key,
            sub_pkg.objid
        );
        let sub_result = pull_sub_pkg_to_named_store(
            client,
            sub_pkg.download_url.as_str(),
            &sub_pkg.objid,
            store_mgr,
            download_options,
        )
        .await?;
        merge_pull_result(&mut result, sub_result);
    }

    result.obj_id = Some(verified.obj_id);
    Ok(result)
}

fn resolve_app_doc_sub_pkg_specs(
    app_doc: &AppDoc,
    app_doc_url: &str,
    download_options: &DownloadTaskOptions,
) -> Result<Vec<SubPkgDownloadSpec>, String> {
    let mut sub_pkgs = Vec::new();

    for (key, sub_pkg) in app_doc.pkg_list.iter() {
        let download_url = sub_pkg
            .source_url
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                sub_pkg.pkg_objid.as_ref().and_then(|objid| {
                    resolve_related_download_url(app_doc_url, objid, download_options).ok()
                })
            })
            .or_else(|| {
                sub_pkg
                    .pkg_objid
                    .as_ref()
                    .map(|objid| format!("cyfs://{}", objid))
            })
            .ok_or_else(|| {
                format!(
                    "AppDoc sub package `{}` missing source_url and pkg_objid",
                    key
                )
            })?;

        let objid = sub_pkg
            .pkg_objid
            .clone()
            .or_else(|| infer_objid_from_url(download_url.as_str()))
            .ok_or_else(|| {
                format!(
                    "AppDoc sub package `{}` does not provide a resolvable objid",
                    key
                )
            })?;

        sub_pkgs.push(SubPkgDownloadSpec {
            key: key.clone(),
            download_url,
            objid,
        });
    }

    Ok(sub_pkgs)
}

fn resolve_related_download_url(
    base_url: &str,
    objid: &ObjId,
    download_options: &DownloadTaskOptions,
) -> Result<String, String> {
    replace_obj_id_in_url(base_url, objid).or_else(|_| {
        build_download_url_from_default_remote(objid, download_options).ok_or_else(|| {
            format!(
                "cannot resolve related download url for {} from {}",
                objid, base_url
            )
        })
    })
}

fn replace_obj_id_in_url(base_url: &str, objid: &ObjId) -> Result<String, String> {
    let parsed_url = url::Url::parse(base_url)
        .map_err(|err| format!("parse url {} failed: {}", base_url, err))?;
    let (base_objid, _) =
        cyfs_get_obj_id_from_url(base_url).map_err(|err| format!("parse objid failed: {}", err))?;
    let mut replaced_url = parsed_url.clone();

    if parsed_url
        .host_str()
        .and_then(|host| ObjId::from_hostname(host).ok())
        .as_ref()
        == Some(&base_objid)
    {
        let host = parsed_url
            .host_str()
            .ok_or_else(|| format!("missing host in {}", base_url))?;
        let mut host_parts = host.split('.').map(str::to_string).collect::<Vec<String>>();
        if host_parts.is_empty() {
            return Err(format!("invalid host {} in {}", host, base_url));
        }
        host_parts[0] = objid.to_base32();
        replaced_url
            .set_host(Some(host_parts.join(".").as_str()))
            .map_err(|_| format!("replace host failed for {}", base_url))?;
        return Ok(replaced_url.to_string());
    }

    let segments = parsed_url
        .path_segments()
        .map(|segments| segments.map(str::to_string).collect::<Vec<String>>())
        .unwrap_or_default();
    for (index, segment) in segments.iter().enumerate() {
        if ObjId::new(segment).ok().as_ref() == Some(&base_objid) {
            let mut new_segments = segments.clone();
            new_segments[index] = objid.to_string();
            replaced_url.set_path(format!("/{}", new_segments.join("/")).as_str());
            return Ok(replaced_url.to_string());
        }
    }

    Err(format!("cannot replace objid in {}", base_url))
}

fn build_download_url_from_default_remote(
    objid: &ObjId,
    download_options: &DownloadTaskOptions,
) -> Option<String> {
    let default_remote_url = download_options
        .default_remote_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let obj_id_in_host = download_options.obj_id_in_host.unwrap_or(false);

    let parsed = url::Url::parse(default_remote_url).ok()?;
    if obj_id_in_host {
        let host = parsed.host_str()?;
        let mut replaced = parsed.clone();
        replaced
            .set_host(Some(format!("{}.{}", objid.to_base32(), host).as_str()))
            .ok()?;
        return Some(replaced.to_string());
    }

    Some(format!(
        "{}/{}",
        default_remote_url.trim_end_matches('/'),
        objid
    ))
}

fn merge_pull_result(total: &mut CyfsPullResult, next: CyfsPullResult) {
    total.total_size = total.total_size.saturating_add(next.total_size);
    total.chunk_count = total.chunk_count.saturating_add(next.chunk_count);
    if total.obj_id.is_none() {
        total.obj_id = next.obj_id;
    }
    for stored_object in next.stored_objects {
        push_stored_object(&mut total.stored_objects, stored_object);
    }
}

fn push_stored_object(stored_objects: &mut Vec<ObjId>, objid: ObjId) {
    if !stored_objects.iter().any(|existing| existing == &objid) {
        stored_objects.push(objid);
    }
}
