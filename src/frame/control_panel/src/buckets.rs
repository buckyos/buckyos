//! The buckets control-panel declares (doc/control_panel/Control Panel Durable Data Schema.md):
//! installed app packages, Agent templates and PIKG staging live in zone buckets of the
//! service; an uninstalled app instance's own buckets are released through nfs-server.

use async_trait::async_trait;
use buckyos_api::bucket_client::{
    AppRelease, Binding, BucketId, ListRequest, PutRequest, PutResult,
};
use buckyos_api::{
    get_buckyos_api_runtime, is_no_such_key, BucketClient, InstallPlan,
    CONTROL_PANEL_PACKAGES_BUCKET, CONTROL_PANEL_PACKAGE_OBJECTS_BUCKET,
    CONTROL_PANEL_SERVICE_NAME,
};
use kRPC::RPCErrors;
use named_store::NamedDataMgr;
use ndn_lib::{ChunkId, FileObject, NamedObject, ObjId};

/// The keys of one control-panel zone bucket (an in-memory stand-in in tests).
#[async_trait]
pub(crate) trait BucketKeys: Send + Sync {
    async fn put(&self, req: PutRequest) -> Result<PutResult, RPCErrors>;
    /// The current binding, or the given version of it; `None` when there is none.
    async fn get(&self, key: &str, version: Option<u64>) -> Result<Option<Binding>, RPCErrors>;
    async fn delete(&self, key: &str) -> Result<(), RPCErrors>;
    async fn list(&self, prefix: &str) -> Result<Vec<Binding>, RPCErrors>;
}

/// `zone/control-panel/_/<name>` through nfs-server.
pub(crate) struct ZoneBucket(pub &'static str);

#[async_trait]
impl BucketKeys for ZoneBucket {
    async fn put(&self, req: PutRequest) -> Result<PutResult, RPCErrors> {
        bucket_client()
            .await?
            .put(&zone_bucket(self.0)?, &req)
            .await
    }

    async fn get(&self, key: &str, version: Option<u64>) -> Result<Option<Binding>, RPCErrors> {
        match bucket_client()
            .await?
            .get(&zone_bucket(self.0)?, key, version)
            .await
        {
            Ok(binding) => Ok(Some(binding)),
            Err(error) if is_no_such_key(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), RPCErrors> {
        unbind(&bucket_client().await?, &zone_bucket(self.0)?, key).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<Binding>, RPCErrors> {
        list_all(&bucket_client().await?, &zone_bucket(self.0)?, prefix).await
    }
}

pub(crate) fn zone_bucket(name: &str) -> Result<BucketId, RPCErrors> {
    BucketId::zone(CONTROL_PANEL_SERVICE_NAME, name)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))
}

pub(crate) async fn bucket_client() -> Result<BucketClient, RPCErrors> {
    get_buckyos_api_runtime()?.get_bucket_client().await
}

/// Every current binding under `prefix`.
pub(crate) async fn list_all(
    client: &BucketClient,
    bucket: &BucketId,
    prefix: &str,
) -> Result<Vec<Binding>, RPCErrors> {
    let mut items = Vec::new();
    let mut start_after = None;
    loop {
        let page = client
            .list_objects(
                bucket,
                &ListRequest {
                    prefix: prefix.to_string(),
                    delimiter: None,
                    start_after: start_after.take(),
                    limit: Some(1000),
                },
            )
            .await?;
        items.extend(page.items);
        match page.next_start_after {
            Some(next) => start_after = Some(next),
            None => return Ok(items),
        }
    }
}

/// Unbinds one key; a key that is already gone is fine.
pub(crate) async fn unbind(
    client: &BucketClient,
    bucket: &BucketId,
    key: &str,
) -> Result<(), RPCErrors> {
    match client.delete(bucket, key, None).await {
        Ok(_) => Ok(()),
        Err(error) if is_no_such_key(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

/// `<owner>/<app_id>/`: every installed version of one app instance.
pub(crate) fn package_prefix(owner: &str, app_id: &str) -> String {
    format!("{owner}/{app_id}/")
}

fn package_key(plan: &InstallPlan, id: &str) -> String {
    format!(
        "{}{}/{id}",
        package_prefix(&plan.owner_user_id, plan.app_instance_id.app_id().as_str()),
        plan.app_doc.version
    )
}

/// A FileObject over one payload, so the `packages` data bucket retains its chunks.
async fn payload_file(
    store: &NamedDataMgr,
    name: &str,
    content: &ObjId,
) -> Result<Option<(ObjId, String)>, RPCErrors> {
    let size = if content.is_chunk_list() {
        if !store.is_object_stored(content, None).await.unwrap_or(false) {
            return Ok(None);
        }
        store
            .open_chunklist_reader(content, 0)
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("open payload {content}: {error}")))?
            .1
    } else {
        let chunk = ChunkId::from_obj_id(content);
        if !store.have_chunk(&chunk).await {
            return Ok(None);
        }
        store
            .query_chunk_state(&chunk)
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("query payload {content}: {error}")))?
            .1
    };
    Ok(Some(
        FileObject::new(name.to_string(), size, content.to_string()).gen_obj_id(),
    ))
}

/// The buckets that retain installed apps: `package_objects` (AppDoc and Package
/// Meta objects) and `packages` (payloads), keyed by app instance and version.
pub(crate) struct PackageBuckets {
    objects: Box<dyn BucketKeys>,
    packages: Box<dyn BucketKeys>,
}

impl PackageBuckets {
    pub(crate) fn zone() -> Self {
        Self {
            objects: Box::new(ZoneBucket(CONTROL_PANEL_PACKAGE_OBJECTS_BUCKET)),
            packages: Box::new(ZoneBucket(CONTROL_PANEL_PACKAGES_BUCKET)),
        }
    }

    /// Binds what an install plan deploys. With `require_all` (after Prepare) a
    /// Package Meta or payload missing from the named store is an error; without
    /// it (end of Acquire) the ones still inside a staged PIKG are bound when
    /// Prepare materializes them.
    pub(crate) async fn retain_plan(
        &self,
        store: &NamedDataMgr,
        plan: &InstallPlan,
        require_all: bool,
    ) -> Result<(), RPCErrors> {
        let missing = |id: &str| {
            RPCErrors::ReasonError(format!(
                "`{id}` of {} is not in the named store",
                plan.app_instance_id
            ))
        };
        let metas = plan
            .selected_packages
            .iter()
            .filter_map(|package| package.package_meta_id.clone());
        for (obj_id, required) in std::iter::once((plan.app.object_id.clone(), false))
            .chain(metas.map(|id| (id, require_all)))
        {
            if !store.is_object_stored(&obj_id, None).await.unwrap_or(false) {
                if required {
                    return Err(missing(&obj_id.to_string()));
                }
                continue;
            }
            self.objects
                .put(PutRequest {
                    key: package_key(plan, &obj_id.to_string()),
                    obj_id: obj_id.to_string(),
                    ..Default::default()
                })
                .await?;
        }

        for content in &plan.required_contents {
            if content.format.as_deref() == Some("named_object") {
                continue;
            }
            let content_id = ObjId::new(&content.content_id).map_err(|error| {
                RPCErrors::ReasonError(format!("payload id `{}`: {error}", content.content_id))
            })?;
            let file = if content_id.is_file_object() {
                store
                    .is_object_stored(&content_id, None)
                    .await
                    .unwrap_or(false)
                    .then(|| (content_id.clone(), None))
            } else {
                let name = content
                    .sub_pkg_name
                    .as_deref()
                    .map(crate::pikg::preferred_archive_name)
                    .unwrap_or_else(|| content.content_id.clone());
                payload_file(store, &name, &content_id)
                    .await?
                    .map(|(obj_id, body)| (obj_id, Some(body)))
            };
            let Some((obj_id, body)) = file else {
                if require_all {
                    return Err(missing(&content.content_id));
                }
                continue;
            };
            if let Some(body) = body {
                store.put_object(&obj_id, &body).await.map_err(|error| {
                    RPCErrors::ReasonError(format!("store payload object {obj_id}: {error}"))
                })?;
            }
            self.packages
                .put(PutRequest {
                    key: package_key(plan, &content.content_id),
                    obj_id: obj_id.to_string(),
                    ..Default::default()
                })
                .await?;
        }
        Ok(())
    }

    /// Unbinds the packages of one app instance, all versions but `keep_version`
    /// (`None`: all of them, the app is uninstalled).
    pub(crate) async fn release(
        &self,
        owner: &str,
        app_id: &str,
        keep_version: Option<&str>,
    ) -> Result<(), RPCErrors> {
        let prefix = package_prefix(owner, app_id);
        for bucket in [&self.packages, &self.objects] {
            for binding in bucket.list(&prefix).await? {
                let version = binding.key[prefix.len()..].split('/').next();
                if keep_version.is_some() && version == keep_version {
                    continue;
                }
                bucket.delete(&binding.key).await?;
            }
        }
        Ok(())
    }
}

/// The app instance is gone: its own buckets are disposed of by data policy.
pub(crate) async fn release_app_buckets(
    owner: &str,
    app_id: &str,
    keep_app_data: bool,
) -> Result<AppRelease, RPCErrors> {
    bucket_client()
        .await?
        .release_app(app_id, owner, keep_app_data)
        .await
}

#[cfg(test)]
#[path = "../../../test/install_plan_fixture.rs"]
mod plan_fixture;

#[cfg(test)]
pub(crate) mod tests {
    use super::plan_fixture as fixture;
    use super::*;
    use std::collections::BTreeMap;

    /// A bucket without versioning: a put replaces the key and bumps its version.
    pub(crate) struct MemoryBucket {
        name: &'static str,
        bindings: std::sync::Mutex<BTreeMap<String, Binding>>,
    }

    impl MemoryBucket {
        pub(crate) fn new(name: &'static str) -> Self {
            Self {
                name,
                bindings: Default::default(),
            }
        }
    }

    #[async_trait]
    impl BucketKeys for MemoryBucket {
        async fn put(&self, req: PutRequest) -> Result<PutResult, RPCErrors> {
            let mut bindings = self.bindings.lock().unwrap();
            let version = bindings.get(&req.key).map_or(1, |b| b.version + 1);
            let created = !bindings.contains_key(&req.key);
            bindings.insert(
                req.key.clone(),
                Binding {
                    bucket_id: zone_bucket(self.name)?,
                    key: req.key,
                    version,
                    is_current: true,
                    is_delete_marker: false,
                    obj_id: Some(req.obj_id.clone()),
                    obj_type: None,
                    size: 0,
                    etag: req.obj_id.clone(),
                    etag_md5: None,
                    content_type: req.content_type,
                    metadata: req.metadata,
                    tags: req.tags,
                    http_headers: req.http_headers,
                    expires_at: req.expires_at,
                    origin: req.origin,
                    last_access_at: None,
                    created_at: 0,
                    created_by: CONTROL_PANEL_SERVICE_NAME.to_string(),
                },
            );
            Ok(PutResult {
                version,
                etag: req.obj_id,
                created,
            })
        }

        async fn get(&self, key: &str, version: Option<u64>) -> Result<Option<Binding>, RPCErrors> {
            Ok(self
                .bindings
                .lock()
                .unwrap()
                .get(key)
                .filter(|b| version.map_or(true, |v| v == b.version))
                .cloned())
        }

        async fn delete(&self, key: &str) -> Result<(), RPCErrors> {
            self.bindings.lock().unwrap().remove(key);
            Ok(())
        }

        async fn list(&self, prefix: &str) -> Result<Vec<Binding>, RPCErrors> {
            Ok(self
                .bindings
                .lock()
                .unwrap()
                .values()
                .filter(|b| b.key.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    #[test]
    fn declared_buckets_pass_the_install_checks() {
        let decls = buckyos_api::control_panel_bucket_decls();
        assert_eq!(
            buckyos_api::validate_app_buckets(
                &decls,
                Some(buckyos_api::CONTROL_PANEL_STORAGE_QUOTA_MIB)
            ),
            Vec::<String>::new()
        );
        assert!(!buckyos_api::validate_app_buckets(&decls, None).is_empty());
    }

    #[tokio::test]
    async fn plan_packages_are_bound_per_instance_and_version() {
        use buckyos_api::{PlannedContent, SelectedPackage};
        let root =
            std::env::temp_dir().join(format!("cp-packages-{}", uuid::Uuid::new_v4().simple()));
        let store = crate::app_staging::tests::test_named_store(&root).await;
        let buckets = PackageBuckets {
            objects: Box::new(MemoryBucket::new(CONTROL_PANEL_PACKAGE_OBJECTS_BUCKET)),
            packages: Box::new(MemoryBucket::new(CONTROL_PANEL_PACKAGES_BUCKET)),
        };
        let mut plan = fixture::plan();
        let (meta_id, meta) = ndn_lib::build_named_object_by_json(
            "pkg",
            &serde_json::json!({"name": "web", "version": "0.1.2"}),
        );
        let payload = b"payload of the web package".repeat(64);
        let chunk = ndn_lib::ChunkHasher::new(None)
            .unwrap()
            .calc_mix_chunk_id_from_bytes(&payload)
            .unwrap();
        plan.selected_packages = vec![SelectedPackage {
            sub_pkg_name: "web".into(),
            pkg_id: "all.web.notes.alice.bns.did#0.1.2".into(),
            package_meta_id: Some(meta_id.clone()),
            docker_image_name: None,
            docker_image_digest: None,
            required: true,
        }];
        plan.required_contents = vec![
            PlannedContent {
                content_id: meta_id.to_string(),
                sub_pkg_name: Some("web".into()),
                package_meta_id: Some(meta_id.clone()),
                expected_docker_image_digest: None,
                format: Some("named_object".into()),
                size: None,
            },
            PlannedContent {
                content_id: chunk.to_string(),
                sub_pkg_name: Some("web".into()),
                package_meta_id: Some(meta_id.clone()),
                expected_docker_image_digest: None,
                format: Some("archive".into()),
                size: Some(payload.len() as u64),
            },
        ];
        store.put_object(&meta_id, &meta).await.unwrap();

        // End of Acquire: the payload is still inside the PIKG.
        buckets.retain_plan(&store, &plan, false).await.unwrap();
        assert!(buckets.retain_plan(&store, &plan, true).await.is_err());
        let prefix = format!("alice/{}/0.1.2/", plan.app_instance_id.app_id());
        assert_eq!(buckets.packages.list(&prefix).await.unwrap().len(), 0);

        store.put_chunk(&chunk, &payload).await.unwrap();
        buckets.retain_plan(&store, &plan, true).await.unwrap();
        let objects: Vec<String> = buckets
            .objects
            .list(&prefix)
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.key)
            .collect();
        assert_eq!(
            objects,
            vec![format!("{prefix}{}", meta_id.to_string())],
            "the AppDoc is bound only when stored"
        );
        let payload_binding = buckets
            .packages
            .get(&format!("{prefix}{}", chunk.to_string()), None)
            .await
            .unwrap()
            .unwrap();
        let file_id = ObjId::new(payload_binding.obj_id.as_deref().unwrap()).unwrap();
        let file: FileObject =
            ndn_lib::load_named_obj(&store.get_object(&file_id).await.unwrap()).unwrap();
        assert_eq!(
            (
                file.content.as_str(),
                file.size,
                file.content_obj.name.as_str()
            ),
            (
                chunk.to_string().as_str(),
                payload.len() as u64,
                "web.tar.gz"
            )
        );

        // An upgrade keeps the running version; an uninstall unbinds all.
        let other = format!(
            "alice/{}/0.1.1/{}",
            plan.app_instance_id.app_id(),
            chunk.to_string()
        );
        buckets
            .packages
            .put(PutRequest {
                key: other.clone(),
                obj_id: file_id.to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        buckets
            .release(
                "alice",
                plan.app_instance_id.app_id().as_str(),
                Some("0.1.2"),
            )
            .await
            .unwrap();
        assert!(buckets.packages.get(&other, None).await.unwrap().is_none());
        assert_eq!(buckets.packages.list(&prefix).await.unwrap().len(), 1);
        buckets
            .release("bob", plan.app_instance_id.app_id().as_str(), None)
            .await
            .unwrap();
        assert_eq!(
            buckets.packages.list(&prefix).await.unwrap().len(),
            1,
            "another user's install is untouched"
        );
        buckets
            .release("alice", plan.app_instance_id.app_id().as_str(), None)
            .await
            .unwrap();
        assert!(buckets.packages.list("").await.unwrap().is_empty());
        assert!(buckets.objects.list("").await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
