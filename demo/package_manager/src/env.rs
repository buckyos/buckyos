use dirs;
use log::*;
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Deserialize, Serialize, Serializer,
};
use serde_json::Value as JsonValue;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use toml::*;

use crate::error::*;
use crate::parser::*;
use crate::version_util;

/*
PackageEnv是一个包管理的环境，下载，安装，加载都在某个env下面进行
一般来说，一个env对应一个工作目录
一个env包含：
index_db: 索引数据库，记录版本与sha256的对应关系
pkg.lock: 记录当前环境下已经安装的包精确信息

在包安装时，需要解析pkg_id，根据pkg_id从 index_db 中获取sha256值，然后下载
安装时，env第一级目录都是直接依赖的包，用npm举例来说，就是package.json中的dependencies，包内部依赖的包以及devDeps不会出现在这里，避免幽灵依赖
env第一级目录中是包的软连接 和 .pkgs 目录，.pkgs目录中才是包的真实内容
所有包的真实内容都在.pkgs的第一级目录中，第一级目录都是类似于 pkg_name#version 这样的文件夹，比如 core#1.0.3，也就是可以同时依赖同名但版本不同的包
在安装时，会先检查是否已经安装了这个包，如果已经安装了，就不再重复安装，如果没有安装，就下载，解压到.pkgs的第一级目录，然后创建软连接
对于包内部的依赖，也都会提升到 .pkgs 的第一级目录中去查找，避免依赖地狱
所以理论上，.pkgs 下面的文件，最多只有2级子目录，第一级是实际的包目录，第二级是包内部依赖的软连接，而软链接又会链接到第一级目录中
这里要考虑的问题就是删除一个包时需要做的操作，理论上 .pkgs 下面的第一级目录只要没有任何软链接指向它，就可以删除，定时删除或者按需删除也可以是一个方案
还有在安装时如果安装失败是否要回退的问题。
 */
#[derive(Debug, Clone)]
pub struct PackageEnv {
    //用来构建env的目录
    pub work_dir: PathBuf,
}

/* MediaInfo是一个包的元信息
  包括pkg_id，
  类型（dir or file）
  完整路径
*/
#[derive(Debug)]
pub enum MediaType {
    Dir,
    File,
}

#[derive(Debug)]
pub struct MediaInfo {
    pub pkg_id: PackageId,
    pub full_path: PathBuf,
    pub media_type: MediaType,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexDB {
    packages: HashMap<String, HashMap<String, PackageMetaInfo>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PackageMetaInfo {
    deps: Vec<String>,
    sha256: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PackageLockInfo {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub dependencies: Vec<PackageLockDeps>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PackageLockDeps {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PackageLockList {
    #[serde(rename = "package")]
    pub packages: Vec<PackageLockInfo>,
}

impl PackageEnv {
    pub fn new(work_dir: PathBuf) -> Self {
        PackageEnv { work_dir }
    }

    pub fn get_work_dir(&self) -> &PathBuf {
        &self.work_dir
    }

    // 检查lock文件是否符合当前package.toml的版本和依赖要求
    pub fn check_lock_need_update(&self) -> PkgSysResult<bool> {
        let package_data = Self::parse_toml(&self.work_dir.join("package.toml"))?;
        let index_data = Self::parse_json(&self.work_dir.join("index.json"))?;
        let lock_file_path = self.work_dir.join("pkg.lock");
        if !lock_file_path.exists() {
            return Ok(true);
        }

        let lock_data = Self::parse_toml(&lock_file_path)?;
        let package_list: PackageLockList = lock_data.try_into().map_err(|err| {
            PackageSystemErrors::ParseError("pkg.lock".to_string(), err.to_string())
        })?;

        if let Some(dependencies) = package_data.get("dependencies").and_then(|d| d.as_table()) {
            for (dep_name, dep_version) in dependencies {
                if !self.check_dependency(
                    dep_name,
                    dep_version.as_str().unwrap(),
                    &package_list,
                    &index_data,
                )? {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn check_dependency(
        &self,
        dep_name: &str,
        dep_version: &str,
        lock_packages: &PackageLockList,
        _index_data: &JsonValue,
    ) -> PkgSysResult<bool> {
        /*这里只查找一层，因为如果顶层的满足条件，那么子依赖也会满足条件
         *因为上次生成lock文件时，子依赖都是根据条件生成的
        （有手动编辑lock文件的可能，先不处理这种情况）
         */
        let mut found = false;
        for lock_info in lock_packages.packages.iter() {
            if lock_info.name == dep_name {
                let lock_version = &lock_info.version;
                if version_util::matches(dep_version, lock_version)? {
                    found = true;

                    // 检查子依赖
                    for lock_sub_dep in &lock_info.dependencies {
                        if !self.check_dependency(
                            &lock_sub_dep.name,
                            &lock_sub_dep.version,
                            lock_packages,
                            _index_data,
                        )? {
                            return Ok(false);
                        }
                    }

                    break;
                }
            }
        }

        if !found {
            info!(
                "Unmatched dependency in lock file name:{}, version:{}",
                dep_name, dep_version
            );
            return Ok(false);
        }

        Ok(true)
    }

    pub fn update_lock_file(&self) -> PkgSysResult<()> {
        let package_data = Self::parse_toml(&self.work_dir.join("package.toml"))?;
        let index_data = Self::parse_json(&self.work_dir.join("index.json"))?;
        let index_db: IndexDB = serde_json::from_value(index_data).map_err(|err| {
            PackageSystemErrors::ParseError("index.json".to_string(), err.to_string())
        })?;

        let mut new_lock_data: Vec<PackageLockInfo> = Vec::new();
        let mut generated = HashSet::new();

        if let Some(dependencies) = package_data.get("dependencies").and_then(|d| d.as_table()) {
            for (dep_name, dep_version) in dependencies {
                let dep_version_str = dep_version.as_str().unwrap();
                self.add_dependency_recursive(
                    &index_db,
                    dep_name,
                    dep_version_str,
                    &mut new_lock_data,
                    &mut generated,
                )?;
            }
        } else {
            info!("No dependencies in package.toml");
        }

        let package_list = PackageLockList {
            packages: new_lock_data,
        };

        let lock_file_path = self.work_dir.join("pkg.lock");
        let new_lock_content = toml::to_string(&package_list).map_err(|err| {
            PackageSystemErrors::UpdateError(format!("Update lock file error: {}", err.to_string()))
        })?;

        fs::write(lock_file_path, new_lock_content)?;

        Ok(())
    }

    fn add_dependency_recursive(
        &self,
        index_db: &IndexDB,
        dep_name: &str,
        dep_version: &str,
        new_lock_data: &mut Vec<PackageLockInfo>,
        generated: &mut HashSet<String>,
    ) -> PkgSysResult<()> {
        let lock_info =
            self.generate_package_lock_info(index_db, &format!("{}#{}", dep_name, dep_version))?;
        let lock_info_str = format!("{}#{}", lock_info.name, lock_info.version);
        if generated.contains(&lock_info_str) {
            debug!("{} already generated, stop", lock_info_str);
            return Ok(());
        }

        generated.insert(lock_info_str.clone());
        new_lock_data.push(lock_info.clone());
        info!("generate lock info: {:?}", lock_info);

        for dep in &lock_info.dependencies {
            self.add_dependency_recursive(
                index_db,
                &dep.name,
                &dep.version,
                new_lock_data,
                generated,
            )?;
        }

        Ok(())
    }

    pub fn generate_package_lock_info(
        &self,
        index_db: &IndexDB,
        pkg_id_str: &str,
    ) -> PkgSysResult<PackageLockInfo> {
        // 只获取一层，不用获取依赖的依赖
        let parser = Parser::new(self.clone());
        let pkg_id = parser.parse(pkg_id_str)?;

        let exact_version = match self.find_exact_version(pkg_id_str, index_db) {
            Ok(version) => version,
            Err(err) => {
                error!("Failed to find exact version for {}: {}", pkg_id_str, err);
                return Err(PackageSystemErrors::VersionNotFoundError(format!(
                    "{:?}",
                    pkg_id
                )));
            }
        };

        info!("get exact version for {}: {}", pkg_id_str, exact_version);

        let package_meta_info = index_db
            .packages
            .get(&pkg_id.name)
            .and_then(|deps| deps.get(&exact_version))
            .ok_or_else(|| {
                PackageSystemErrors::VersionNotFoundError(format!(
                    "Version {} not found for package {}",
                    exact_version, pkg_id.name
                ))
            })?;

        info!("get meta info for {}: {:?}", pkg_id_str, package_meta_info);

        let mut lock_info = PackageLockInfo {
            name: pkg_id.name.clone(),
            version: exact_version.clone(),
            sha256: package_meta_info.sha256.clone(),
            dependencies: Vec::new(),
        };

        for dep in &package_meta_info.deps {
            match self.find_exact_version(dep, index_db) {
                Ok(version) => {
                    info!("get exact version for {}: {}", dep, version);
                    let package_id = parser.parse(dep)?;
                    lock_info.dependencies.push(PackageLockDeps {
                        name: package_id.name.clone(),
                        version: version.clone(),
                    });
                }
                Err(err) => {
                    let err_msg = format!(
                        "Failed to find exact version for dep: {}, err: {}",
                        dep,
                        err.to_string()
                    );
                    error!("{}", err_msg);
                    return Err(PackageSystemErrors::VersionNotFoundError(err_msg));
                }
            };
        }

        Ok(lock_info)
    }

    pub fn find_exact_version(&self, pkg_id_str: &str, index_db: &IndexDB) -> PkgSysResult<String> {
        let parser: Parser = Parser::new(self.clone());
        let pkg_id = parser.parse(pkg_id_str)?;

        if let Some(pkg_list) = index_db.packages.get(&pkg_id.name) {
            // 将pkg_deps的key组成Vec，并从大到小排序
            let mut versions: Vec<String> = pkg_list.keys().cloned().collect();
            if versions.is_empty() {
                return Err(PackageSystemErrors::VersionNotFoundError(format!(
                    "{:?}",
                    pkg_id
                )));
            }
            // 理论上不应该出现重合的，所以不处理Ge和Le，Ne，版本高的排在前面
            versions.sort_by(|a, b| {
                version_util::compare(a, b)
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        Ordering::Equal
                    })
                    .reverse()
            });
            debug!("sort versions for {}: {:?}", pkg_id.name, versions);

            // TODO 这里还要处理用sha256标明版本的情况
            self.get_matched_version(&pkg_id, &versions)
        } else {
            Err(PackageSystemErrors::VersionNotFoundError(format!(
                "{:?}",
                pkg_id
            )))
        }
    }

    pub async fn get_deps(&self, pkg_id: &str, update: bool) -> PkgSysResult<Vec<PackageId>> {
        /* 先看env中是否有index.db (暂时只用一个json文件代替)，如果有，直接从index.db中获取依赖关系
         * 如果没有，看看%user%/buckyos/index下是否有index.db，如果有，从中获取依赖关系
         * 如果没有，创建相应目录并且下载index.db，然后从中获取依赖关系
         */
        /* 为了简单，现在就链式解析下来
         * 实际实现时，应该解析出一个依赖树，
         * 然后递归解析，找出共有和兼容依赖，减少下载和安装次数
         */
        let index = self.get_index(update).await?;

        let mut deps: Vec<PackageId> = vec![];
        let mut parsed = HashSet::new();

        self.get_deps_impl(&pkg_id, &index, &mut deps, &mut parsed)?;

        Ok(deps)
    }

    // 递归获取，获取到的依赖放到result中
    // 结果每一个都是精确的版本号，不应该有>=,<=,>,<等，version一定是有值的，否则就是解析失败
    fn get_deps_impl(
        &self,
        pkg_id_str: &str,
        index_db: &IndexDB,
        result: &mut Vec<PackageId>,
        parsed: &mut HashSet<String>,
    ) -> PkgSysResult<()> {
        let parser = Parser::new(self.clone());
        // 这里判断是否已经获取过了，避免出现环
        // if parsed.contains(pkg_id_str) {
        //     debug!("{} already parsed. stop", pkg_id_str);
        //     return Ok(());
        // }
        // parsed.insert(pkg_id_str.to_string());

        let pkg_id = parser.parse(pkg_id_str)?;

        if let Some(pkg_list) = index_db.packages.get(&pkg_id.name) {
            // 将pkg_deps的key组成Vec，并从大到小排序
            let mut versions: Vec<String> = pkg_list.keys().cloned().collect();
            if versions.is_empty() {
                return Err(PackageSystemErrors::VersionNotFoundError(format!(
                    "{:?}",
                    pkg_id
                )));
            }
            // 理论上不应该出现重合的，所以不处理Ge和Le，Ne，版本高的排在前面
            versions.sort_by(|a, b| {
                version_util::compare(a, b)
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        Ordering::Equal
                    })
                    .reverse()
            });
            debug!("sort versions for {}: {:?}", pkg_id.name, versions);

            let matched_version = match self.get_matched_version(&pkg_id, &versions) {
                Ok(version) => version,
                Err(err) => {
                    error!(
                        "Failed to get matched version for {}, all versions: {:?}",
                        pkg_id_str, versions
                    );
                    return Err(err);
                }
            };
            let exact_pkg_id_str = format!("{}#{}", pkg_id.name, matched_version);

            // 如果精确的版本号存在，就不再递归
            if parsed.contains(&exact_pkg_id_str) {
                debug!("{} already parsed. stop.", exact_pkg_id_str);
                return Ok(());
            }
            info!("get deps {} => {}", pkg_id_str, exact_pkg_id_str);
            parsed.insert(exact_pkg_id_str);

            let package_meta_info = pkg_list.get(&matched_version).ok_or_else(|| {
                PackageSystemErrors::VersionNotFoundError(format!(
                    "Version {} not found for package {}",
                    matched_version, pkg_id.name
                ))
            })?;

            result.push(PackageId {
                name: pkg_id.name.clone(),
                version: Some(matched_version.clone()),
                sha256: Some(package_meta_info.sha256.clone()),
            });

            // 获取依赖，然后递归的获取依赖的依赖
            for dep in &package_meta_info.deps {
                self.get_deps_impl(dep, index_db, result, parsed)?;
            }
        }

        Ok(())
    }

    fn get_matched_version(&self, pkg_id: &PackageId, versions: &[String]) -> PkgSysResult<String> {
        if versions.is_empty() {
            return Err(PackageSystemErrors::VersionNotFoundError(format!(
                "{:?}",
                pkg_id
            )));
        }

        // 如果有sha，优先用sha，否则用version
        if let Some(sha256) = &pkg_id.sha256 {
            for v in versions {
                if v.eq(sha256) {
                    return Ok(v.to_string());
                }
            }

            return Err(PackageSystemErrors::VersionNotFoundError(format!(
                "{:?}",
                pkg_id
            )));
        }

        if let Some(version) = &pkg_id.version {
            let ret = version_util::find_matched_version(version, &versions);
            debug!("find matched version for {:?} => {:?}", pkg_id, ret);
            return ret;
        }

        // 返回列表里的第一个
        if let Some(v) = versions.get(0) {
            return Ok(v.to_string());
        }

        Err(PackageSystemErrors::VersionNotFoundError(format!(
            "{:?}",
            pkg_id
        )))
    }

    pub async fn get_index(&self, update: bool) -> PkgSysResult<IndexDB> {
        //TODO env是否应该有自己的index？
        /*let user_dir =
            dirs::home_dir().ok_or(PackageSystemErrors::UnknownError("No home dir".to_string()))?;
        let global_index_file = user_dir.join("buckyos/index/index.json");*/

        let global_index_file = self.work_dir.join("index.json");

        if update || !global_index_file.exists() {
            self.update_index().await?;
        }

        let index_str = std::fs::read_to_string(global_index_file)?;
        let index_db: IndexDB = serde_json::from_str(&index_str).map_err(|err| {
            PackageSystemErrors::ParseError("index.json".to_string(), err.to_string())
        })?;

        Ok(index_db)
    }

    async fn update_index(&self) -> PkgSysResult<()> {
        //update只更新global的index，这里index只是一个文件
        //实际在实现时，index应该是一组文件，按需更新
        /*let user_dir =
            dirs::home_dir().ok_or(PackageSystemErrors::UnknownError("No home dir".to_string()))?;
        //创建目录
        let global_index_dir = user_dir.join("buckyos/index");
        std::fs::create_dir_all(global_index_dir)?;
        //下载index.json
        let index_url = "https://buckyos.com/index.json";
        let index_str = reqwest::get(index_url).await?.text().await?;
        let global_index_file = global_index_dir.join("index.json");
        std::fs::write(global_index_file, index_str)?;*/

        Ok(())
    }

    fn parse_toml(file_path: &PathBuf) -> PkgSysResult<Value> {
        let content = fs::read_to_string(file_path)?;
        let value = content.parse::<Value>().map_err(|err| {
            PackageSystemErrors::ParseError(
                file_path.to_string_lossy().to_string(),
                err.to_string(),
            )
        })?;
        Ok(value)
    }

    fn parse_json(file_path: &PathBuf) -> PkgSysResult<JsonValue> {
        let content = fs::read_to_string(file_path)?;
        let value = serde_json::from_str(&content).map_err(|err| {
            PackageSystemErrors::ParseError(
                file_path.to_string_lossy().to_string(),
                err.to_string(),
            )
        })?;
        Ok(value)
    }
}

/*
Env目录中有一个简化的index.json，记录了包的依赖关系
index.json的简化设计：
{
    "packages": {
        "a": {
            "1.0.2": {
                "deps": ["b#>2.0", "c#1.0.1"],
                "sha256": "1234567890"
            },
            "1.0.1": {
                "deps": ["b", "c#<1.0.1"],
                "sha256": "1234567890"

            }
        },
        "b": {
            "2.0": {
                "deps": ["d#>3.0"],
                "sha256": "1234567890"
            },
            "1.0": {
                "deps": ["d#<=3.0"],
                "sha256": "1234567890"W
            }
        },
        "c": {
            "1.0.1": {
                "deps": [],
                "sha256": "1234567890"
            }
        },
        "d": {
            "3.0.1": {
                "deps": [],
                "sha256": "1234567890"
            },
            "3.0.0": {
                "deps": [],
                "sha256": "1234567890"
            }
        }
    },
    ....
}

有一个简化的pkg.lock，记录了当前环境下已经安装的包精确信息，是toml格式
内容类似为：

[[package]]
name = "a"
version = "1.0.2"
sha256 = "1234567890"
# 子依赖
dependencies = [
    { name = "b", version = "2.0.1" },
    { name = "c", version = "1.0.1" }
]

[[package]]
name = "b"
version = "2.0.1"
sha256 = "1234567890"
dependencies = [
    { name = "c", version = "2.0.1" }
]

[[package]]
name = "c"
version = "1.0.1"
sha256 = "1234567890"
dependencies = []

[[package]]
name = "c"
version = "2.0.1"
sha256 = "1234567890"
dependencies = []


还有一个简化的package.toml, 记录了当前环境依赖的包及其他信息
内容类似为：

[package]
name = "my_project"
version = "1.0.0"

[dependencies]
a = ">1.0.1"
b = "2.0.1"

 */

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_update_lock_file() {
        let dir = tempdir().unwrap();
        let work_dir = dir.path().to_path_buf();

        // Create a mock package.toml
        let package_toml_content = r#"
            [package]
            name = "my_project"
            version = "1.0.0"

            [dependencies]
            a = ">1.0.1"
            b = "2.0.1"
        "#;
        fs::write(work_dir.join("package.toml"), package_toml_content).unwrap();

        // Create a mock index.json
        let index_json_content = r#"
            {
                "packages": {
                    "a": {
                        "1.0.2": {
                            "deps": ["b#2.0.1", "c#1.0.1"],
                            "sha256": "1234567890"
                        }
                    },
                    "b": {
                        "2.0.1": {
                            "deps": ["c#2.0.1"],
                            "sha256": "0987654321"
                        }
                    },
                    "c": {
                        "1.0.1": {
                            "deps": [],
                            "sha256": "1122334455"
                        },
                        "2.0.1": {
                            "deps": [],
                            "sha256": "5566778899"
                        }
                    }
                }
            }
        "#;
        fs::write(work_dir.join("index.json"), index_json_content).unwrap();

        let env = PackageEnv::new(work_dir.clone());

        env.update_lock_file().unwrap();

        let lock_file_path = work_dir.join("pkg.lock");
        assert!(lock_file_path.exists());

        let lock_content = fs::read_to_string(lock_file_path).unwrap();
        let lock_data: PackageLockList = toml::from_str(&lock_content).unwrap();

        assert_eq!(lock_data.packages.len(), 4);

        let package_a = &lock_data.packages[0];
        assert_eq!(package_a.name, "a");
        assert_eq!(package_a.version, "1.0.2");
        assert_eq!(package_a.sha256, "1234567890");
        assert_eq!(package_a.dependencies.len(), 2);
        assert_eq!(package_a.dependencies[0].name, "b");
        assert_eq!(package_a.dependencies[0].version, "2.0.1");
        assert_eq!(package_a.dependencies[1].name, "c");
        assert_eq!(package_a.dependencies[1].version, "1.0.1");

        let package_b = &lock_data.packages[1];
        assert_eq!(package_b.name, "b");
        assert_eq!(package_b.version, "2.0.1");
        assert_eq!(package_b.sha256, "0987654321");
        assert_eq!(package_b.dependencies.len(), 1);
        assert_eq!(package_b.dependencies[0].name, "c");
        assert_eq!(package_b.dependencies[0].version, "2.0.1");

        let package_c = &lock_data.packages[2];
        assert_eq!(package_c.name, "c");
        assert_eq!(package_c.version, "2.0.1");
        assert_eq!(package_c.sha256, "5566778899");
        assert_eq!(package_c.dependencies.len(), 0);

        let package_c2 = &lock_data.packages[3];
        assert_eq!(package_c2.name, "c");
        assert_eq!(package_c2.version, "1.0.1");
        assert_eq!(package_c2.sha256, "1122334455");
        assert_eq!(package_c2.dependencies.len(), 0);
    }

    #[test]
    fn test_check_lock_need_update() {
        let dir = tempdir().unwrap();
        let work_dir = dir.path().to_path_buf();

        // Create a mock package.toml
        let package_toml_content = r#"
            [package]
            name = "my_project"
            version = "1.0.0"

            [dependencies]
            a = ">1.0.1"
            b = "2.0.1"
        "#;
        fs::write(work_dir.join("package.toml"), package_toml_content).unwrap();

        // Create a mock index.json
        let index_json_content = r#"
            {
                "packages": {
                    "a": {
                        "1.0.2": {
                            "deps": ["b#2.0.1", "c#1.0.1"],
                            "sha256": "1234567890"
                        }
                    },
                    "b": {
                        "2.0.1": {
                            "deps": ["c#2.0.1"],
                            "sha256": "0987654321"
                        }
                    },
                    "c": {
                        "1.0.1": {
                            "deps": [],
                            "sha256": "1122334455"
                        },
                        "2.0.1": {
                            "deps": [],
                            "sha256": "5566778899"
                        }
                    }
                }
            }
        "#;
        fs::write(work_dir.join("index.json"), index_json_content).unwrap();

        // Create a mock pkg.lock
        let lock_toml_content = r#"
            [[package]]
            name = "a"
            version = "1.0.2"
            sha256 = "1234567890"
            dependencies = [
                { name = "b", version = "2.0.1" },
                { name = "c", version = "1.0.1" }
            ]

            [[package]]
            name = "b"
            version = "2.0.1"
            sha256 = "0987654321"
            dependencies = [
                { name = "c", version = "2.0.1" }
            ]

            [[package]]
            name = "c"
            version = "1.0.1"
            sha256 = "1122334455"
            dependencies = []

            [[package]]
            name = "c"
            version = "2.0.1"
            sha256 = "5566778899"
            dependencies = []
        "#;
        fs::write(work_dir.join("pkg.lock"), lock_toml_content).unwrap();

        let env = PackageEnv::new(work_dir.clone());
        let need_update = env.check_lock_need_update().unwrap();
        assert!(!need_update);
    }
}
