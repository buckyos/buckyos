use crate::error::{RepoError, RepoResult};
use log::*;
use ndn_lib::ChunkId;
use package_lib::{IndexStore, PackageMeta, Verifier as PkgVerifier};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn time_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn is_valid_chunk_id(chunk_id: &str) -> bool {
    match ChunkId::new(chunk_id) {
        Ok(_) => true,
        Err(_) => false,
    }
}

/*
   管理两个IndexStore，一个是本地的index，负责zone内发布，删除包，称为zone_index。
   另一个是从index server上同步回的net_index，只可查询，不可修改
   查找包时：
   1.从外部收到包查询的请求时（即第一层查询），先从zone_index上查找，没有找到再去net_index上查找
   2.当包有依赖时，递归查找依赖包。从第二层查询开始，按照3，4规则查找
   3.如果包信息在zone_index上，那么可以在zone_index上以及net_index上继续查找其依赖包
   4.如果包信息在net_index上，那么只能在net_index上查找其依赖包，不能再去zone_index上查找
*/

pub struct PackageRepository {
    pub net_index_store: IndexStore,
    pub zone_index_store: IndexStore,
}

impl PackageRepository {
    pub fn new(net_index_store: IndexStore, zone_index_store: IndexStore) -> Self {
        PackageRepository {
            net_index_store,
            zone_index_store,
        }
    }

    pub async fn add_package(
        &self,
        name: &str,
        version: &str,
        author: &str,
        chunk_id: &str,
        dependencies: &HashMap<String, String>,
        sign: &str,
    ) -> RepoResult<()> {
        // TODO: check chunk_id exists in chunk store
        let pkg_meta = PackageMeta {
            name: name.to_string(),
            version: version.to_string(),
            author: author.to_string(),
            chunk_id: chunk_id.to_string(),
            dependencies: serde_json::to_value(dependencies).map_err(|e| {
                error!("dependencies to_value failed: {:?}", e);
                RepoError::ParamError(format!(
                    "dependencies to_value failed, dep:{:?} err:{:?}",
                    dependencies, e
                ))
            })?,
            sign: sign.to_string(),
            pub_time: time_now(),
        };
        self.zone_index_store
            .insert_pkg_meta(&pkg_meta)
            .await
            .map_err(|e| {
                error!("insert_pkg_meta failed: {:?}", e);
                RepoError::DbError(e.to_string())
            })?;
        Ok(())
    }

    // version_desc version or chunk_id
    pub fn remove_package(&self, name: &str, version_desc: &str) -> RepoResult<()> {
        let is_chunk_id = is_valid_chunk_id(version_desc);
        self.zone_index_store
            .remove_pkg_meta(name, version_desc, is_chunk_id)
            .map_err(|e| {
                error!("remove_pkg_meta_with_chunk_id failed: {:?}", e);
                RepoError::DbError(e.to_string())
            })?;
        Ok(())
    }

    // return (meta_info, is_stable), 只有meta_info不为None时，is_stable才有意义
    pub fn get_package_meta(
        &self,
        name: &str,
        version_desc: &str,
        need_stable: bool, // 是否只查找稳定版本, true: 只能从net_index查找，false: 优先从zone_index查找
    ) -> RepoResult<(Option<PackageMeta>, bool)> {
        let mut meta_info = None;
        let is_chunk_id = is_valid_chunk_id(version_desc);
        if !need_stable {
            meta_info = self
                .zone_index_store
                .get_pkg_meta(name, version_desc, is_chunk_id)
                .map_err(|e| {
                    error!("get_pkg_meta failed: {:?}", e);
                    RepoError::DbError(e.to_string())
                })?;
        }
        if meta_info.is_none() {
            let net_meta_info = self
                .net_index_store
                .get_pkg_meta(name, version_desc, is_chunk_id)
                .map_err(|e| {
                    error!("get_pkg_meta failed: {:?}", e);
                    RepoError::DbError(e.to_string())
                })?;
            Ok((net_meta_info, true))
        } else {
            Ok((meta_info, false))
        }
    }

    pub fn resolve_dependencies(
        &self,
        pkg_meta: &PackageMeta,
        need_stable: bool,
    ) -> RepoResult<Vec<PackageMeta>> {
        let mut dependencies = Vec::new();
        let deps: Value = pkg_meta.dependencies.clone();
        let deps: HashMap<String, String> = serde_json::from_value(deps.clone()).map_err(|e| {
            error!("dependencies from_value failed: {:?}", e);
            RepoError::ParamError(format!(
                "dependencies from_value failed, deps:{:?} err:{:?}",
                deps, e
            ))
        })?;
        for (dep_name, dep_version) in deps.iter() {
            let (dep_meta_info, is_stable) =
                self.get_package_meta(dep_name, dep_version, need_stable)?;
            if let Some(dep_meta_info) = dep_meta_info {
                dependencies.push(dep_meta_info.clone());
                let dep_deps = self.resolve_dependencies(&dep_meta_info, is_stable)?;
                for dep_dep in dep_deps.iter() {
                    dependencies.push(dep_dep.clone());
                }
            }
        }
        Ok(dependencies)
    }

    // 准备一个包，会将包及其所有的依赖包放入chunk store中，然后返回包的元信息和所有依赖包的元信息
    pub async fn prepare_pkg(
        &self,
        name: &str,
        version_desc: &str,
    ) -> RepoResult<(PackageMeta, Vec<PackageMeta>)> {
        // TODO: 先同步的解决所有依赖
        let (meta_info, is_stable) = self.get_package_meta(name, version_desc, false)?;
        if meta_info.is_none() {
            return Err(RepoError::NotFound(format!(
                "package not found, name:{}, version:{}",
                name, version_desc
            )));
        }
        let meta_info = meta_info.unwrap();
        self.pull_pkg(&meta_info).await?;
        let dependencies = self.resolve_dependencies(&meta_info, is_stable)?;
        //依次下载所有的包到chunk里
        for dep in dependencies.iter() {
            self.pull_pkg(dep).await?;
        }

        Ok((meta_info, dependencies))
    }

    pub async fn pull_pkg(&self, meta_info: &PackageMeta) -> RepoResult<()> {
        if let Err(e) =
            PkgVerifier::verify(&meta_info.author, &meta_info.chunk_id, &meta_info.sign).await
        {
            return Err(RepoError::VerifyError(format!(
                "verify failed, meta:{:?}, err:{}",
                meta_info, e
            )));
        }
        if self.check_exist(meta_info).await? {
            return Ok(());
        }
        unimplemented!("pull from other zone")
    }

    pub async fn check_exist(&self, meta_info: &PackageMeta) -> RepoResult<bool> {
        //TODO: 通过chunk manager查询chunk是否存在
        unimplemented!("check_exist")
    }
}
