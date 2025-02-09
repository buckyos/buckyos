use dirs;
use flate2::read::GzDecoder;
use futures::future::join_all;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt::format;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tar::Archive;
use tokio::sync::oneshot;
use tokio::time::Duration;
use toml::*;

use crate::downloader::{self, *};
use crate::error::*;
use crate::parser::*;
use crate::version_util::*;

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
#[derive(Debug, Clone)]
pub enum MediaType {
    Dir,
    File,
}

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub pkg_id: PackageId,
    pub full_path: PathBuf,
    pub media_type: MediaType,
}

#[derive(Serialize, Deserialize, Debug)]
struct PackageMetaInfo {
    deps: HashMap<String, String>,
    sha256: String,
    author_did: String,
    author_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageLockInfo {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub dependencies: Vec<PackageLockDeps>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageLockDeps {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageLockList {
    #[serde(rename = "package")]
    pub packages: Vec<PackageLockInfo>,
}

/**
 * env下有一个meta文件夹，用来存放当前env已知的所有包的元信息
 * meta文件夹下都是一个个元文件，命名方式为: pkg_name#version#sha256-xxxx，因为这里都是已经获取确定信息的包了，所以信息是完备的
 * 文件内容为json，包含该pkg的deps及其他信息
 */

#[allow(dead_code)]
impl PackageEnv {
    pub fn new(work_dir: PathBuf) -> Self {
        PackageEnv { work_dir }
    }

    pub fn get_work_dir(&self) -> PathBuf {
        self.work_dir.clone()
    }

    pub fn get_meta_dir(&self) -> PathBuf {
        let meta_dir = self.work_dir.join("meta");
        if !meta_dir.exists() {
            fs::create_dir(&meta_dir).unwrap();
        }
        meta_dir
    }

    pub fn get_install_dir(&self) -> PathBuf {
        self.work_dir.clone()
    }

    pub fn get_cache_dir(&self) -> PathBuf {
        let cache_dir = self.work_dir.join("cache");
        if !cache_dir.exists() {
            fs::create_dir(&cache_dir).unwrap();
        }
        cache_dir
    }

    pub fn write_meta_file(
        &self,
        pkg_id: &PackageId,
        deps: &HashMap<String, String>,
        author_did: &str,
        author_name: &str,
    ) -> PkgResult<()> {
        debug!(
            "write_meta_file: {:?}, deps:{:?}, author_did:{}, author_name:{}",
            pkg_id, deps, author_did, author_name
        );
        let meta_dir = self.get_meta_dir();
        let meta_file_name = format!(
            "{}#{}#{}",
            pkg_id.name,
            pkg_id.version.as_ref().unwrap(),
            pkg_id.sha256.as_ref().unwrap().replace(":", "-")
        );
        let meta_file = meta_dir.join(meta_file_name);

        let meta_info = PackageMetaInfo {
            deps: deps.clone(),
            sha256: pkg_id.sha256.as_ref().unwrap().to_string(),
            author_did: author_did.to_string(),
            author_name: author_name.to_string(),
        };

        let meta_content = serde_json::to_string(&meta_info)?;
        let mut file = fs::File::create(meta_file)?;
        file.write_all(meta_content.as_bytes())?;

        Ok(())
    }

    pub fn get_exact_pkg_id(&self, pkg_id_str: &str) -> PkgResult<Option<PackageId>> {
        let pkg_id = Parser::parse(pkg_id_str)?;
        let meta_dir = self.get_meta_dir();
        //遍历meta文件夹下的所有文件，找到所有pkg_name与pkg_id.name相同的文件，并解析出name, version, sha256
        let meta_files: Vec<PathBuf> = fs::read_dir(&meta_dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let entry_path = entry.path();
                if entry_path.is_file() {
                    if let Some(file_name) = entry_path.file_name() {
                        if file_name.to_string_lossy().starts_with(&pkg_id.name) {
                            return Some(entry_path);
                        }
                    }
                }
                None
            })
            .collect();

        //解析meta_files的文件名，获得name, version, sha256
        let mut found_pkg = None;
        for meta_file in meta_files {
            let file_name = meta_file.file_name().unwrap().to_string_lossy();
            //第一部分是name，中间的是version,最后一部分是sha256，要考虑version中有#的情况
            let file_name_parts: Vec<&str> = file_name.split('#').collect();
            if file_name_parts.len() != 3 {
                continue;
            }
            let file_name_len = file_name_parts.len();
            let name = file_name_parts[0].to_string();
            let version = file_name_parts[1..file_name_len - 1].join("#");
            let sha256 = file_name_parts[file_name_len - 1].to_string();
            let sha256 = sha256.replace("-", ":");

            if name == pkg_id.name {
                if let Some(sha256) = &pkg_id.sha256 {
                    if sha256 == sha256 {
                        found_pkg = Some(PackageId {
                            name,
                            version: Some(version.to_string()),
                            sha256: Some(sha256.to_string()),
                        });
                        break;
                    }
                } else if let Some(pkg_version) = &pkg_id.version {
                    if VersionUtil::matches(pkg_version, &version)? {
                        found_pkg = Some(PackageId {
                            name,
                            version: Some(version.to_string()),
                            sha256: Some(sha256.to_string()),
                        });
                        break;
                    }
                }
            }
        }

        debug!("found_pkg: {} => {:?}", pkg_id_str, found_pkg);

        Ok(found_pkg)
    }

    // Each of the returned results is with an exact version.
    pub fn get_deps(&self, pkg_id_str: &str) -> PkgResult<Vec<PackageId>> {
        let mut deps: Vec<PackageId> = vec![];
        let mut visited = HashSet::new();

        self.get_deps_impl(&pkg_id_str, &mut deps, &mut visited)?;

        Ok(deps)
    }

    // find all deps(in dep_dir) for a package, the dep is exact version, and all deps are in the form of package_name#version
    // and all deps desc file is in dep_dir with the name of package_name#version
    pub fn get_deps_impl(
        &self,
        pkg_id_str: &str,
        deps: &mut Vec<PackageId>,
        visited: &mut HashSet<String>,
    ) -> PkgResult<()> {
        if visited.contains(pkg_id_str) {
            return Ok(());
        }

        let pkg_id = self.get_exact_pkg_id(pkg_id_str)?;

        if pkg_id.is_none() {
            return Err(PkgError::VersionNotFoundError(pkg_id_str.to_owned()));
        }

        let pkg_id = pkg_id.unwrap();
        let meta_file_name = format!(
            "{}#{}#{}",
            pkg_id.name,
            pkg_id.version.as_ref().unwrap(),
            pkg_id.sha256.as_ref().unwrap().replace(":", "-")
        );
        visited.insert(meta_file_name.clone());
        deps.push(pkg_id.clone());

        let meta_file = self.get_meta_dir().join(meta_file_name);

        let meta_content = fs::read_to_string(meta_file)?;
        let meta_json: PackageMetaInfo = serde_json::from_str(&meta_content)?;

        for (dep_name, dep_version) in meta_json.deps.iter() {
            let dep_id_str = format!("{}#{}", dep_name, dep_version);
            self.get_deps_impl(&dep_id_str, deps, visited)?;
        }

        Ok(())
    }

    pub fn load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        match self.load_strictly(pkg_id_str) {
            Ok(media_info) => {
                info!("load strictly {} => {:?}", pkg_id_str, media_info);
                Ok(media_info)
            }
            Err(_) => {
                let ret = self.try_load(pkg_id_str);
                debug!("try load {} => {:?}", pkg_id_str, ret);
                ret
            }
        }
    }

    pub fn load_strictly(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        let pkg_id = self.get_exact_pkg_id(pkg_id_str)?;
        if pkg_id.is_none() {
            return Err(PkgError::LoadError(
                pkg_id_str.to_owned(),
                "Not found".to_owned(),
            ));
        }

        let pkg_id = pkg_id.unwrap();

        let target_pkg = format!("{}#{}", pkg_id.name, pkg_id.version.as_ref().unwrap());

        let target_path = self.get_install_dir().join(target_pkg.clone());
        if target_path.exists() {
            let media_type = if target_path.is_dir() {
                MediaType::Dir
            } else {
                MediaType::File
            };

            Ok(MediaInfo {
                pkg_id,
                full_path: target_path,
                media_type,
            })
        } else {
            Err(PkgError::LoadError(
                pkg_id_str.to_owned(),
                format!("Package file not found: {}", target_path.display()),
            ))
        }
    }

    pub fn try_load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        //尽力加载，判断install目录中是否有目标包，如果有就加载
        let pkg_id = Parser::parse(pkg_id_str)?;
        if let Some(ref sha256) = pkg_id.sha256 {
            let target_pkg = format!("{}#{}", pkg_id.name, sha256.replace(":", "-"));
            let target_path = self.get_install_dir().join(target_pkg);
            if target_path.exists() {
                let media_type = if target_path.is_dir() {
                    MediaType::Dir
                } else {
                    MediaType::File
                };

                return Ok(MediaInfo {
                    pkg_id,
                    full_path: target_path,
                    media_type,
                });
            }
        } else if let Some(ref pkg_version) = pkg_id.version {
            //遍历install文件夹下的所有文件，找到所有pkg_name与pkg_id.name相同的文件，并解析出name, Version
            let install_dir = self.get_install_dir();
            let valid_file: Vec<PathBuf> = fs::read_dir(&install_dir)?
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let entry_path = entry.path();
                    if let Some(file_name) = entry_path.file_name() {
                        if file_name.to_string_lossy().starts_with(&pkg_id.name) {
                            return Some(entry_path);
                        }
                    }
                    None
                })
                .collect();

            for file in valid_file {
                let file_name = file.file_name().unwrap().to_string_lossy();
                let mut file_name_parts: Vec<&str> = file_name.split('#').collect();
                if file_name_parts.len() < 1 {
                    return Err(PkgError::ParseError(
                        file.to_string_lossy().to_string(),
                        "Invalid file name".to_string(),
                    ));
                }
                if file_name_parts.len() == 1 {
                    file_name_parts.append(&mut vec!["*"]);
                }
                let file_name_len = file_name_parts.len();
                let name = file_name_parts[0].to_string();
                let version = file_name_parts[1..file_name_len].join("#");
                if name == pkg_id.name {
                    if VersionUtil::matches(&pkg_version, &version)? {
                        let media_type = if file.is_dir() {
                            MediaType::Dir
                        } else {
                            MediaType::File
                        };
                        let target_pkg_id = PackageId {
                            name,
                            version: Some(version.to_string()),
                            sha256: None,
                        };
                        return Ok(MediaInfo {
                            pkg_id: target_pkg_id,
                            full_path: file,
                            media_type,
                        });
                    }
                }
            }
        }

        Err(PkgError::LoadError(
            pkg_id_str.to_owned(),
            "Package not found".to_owned(),
        ))
    }

    pub fn check_pkg_ready(&self, pkg_id_str: &str) -> PkgResult<Vec<PackageId>> {
        //获取pkg的依赖，依次检查依赖是否已经安装
        let deps: Vec<PackageId> = match self.get_deps(pkg_id_str) {
            Ok(deps) => deps,
            Err(e) => {
                info!("check package ready failed. error: {}", e);
                return Err(e);
            }
        };
        for dep in &deps {
            let target_pkg = format!("{}#{}", dep.name, dep.version.as_ref().unwrap());
            let target_path = self.get_install_dir().join(target_pkg);
            if !target_path.exists() {
                info!(
                    "Dependency not found: {}, pkg is not ready!",
                    target_path.display()
                );
                return Err(PkgError::LoadError(
                    pkg_id_str.to_owned(),
                    format!("Dependency not found: {}", target_path.display()),
                ));
            }
        }

        debug!("Package is ready: {}", pkg_id_str);

        Ok(deps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_pkg_load_strictly() {
        //创建一个临时目录
        let tmp_dir = tempdir().unwrap();
        let env = PackageEnv::new(tmp_dir.path().to_path_buf());
        //创建meta目录
        let meta_dir = env.get_meta_dir();
        fs::create_dir(&meta_dir).unwrap();

        let meta_file_name = "a#0.1.0#sha256xxxx";
        //创建一个meta文件
        let meta_file = env.get_meta_dir().join(meta_file_name);
        //写入meta文件内容
        let meta_content = r#"{"deps": {}, "sha256": "sha256xxxx"}"#;
        fs::write(&meta_file, meta_content).unwrap();

        //创建一个a#0.1.0的文件夹
        let pkg_dir = env.get_install_dir().join("a#0.1.0");
        fs::create_dir(&pkg_dir).unwrap();

        //测试load
        let media_info = env.load_strictly("a#0.1.0").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        //测试get_deps
        let deps = env.get_deps("a#0.1.0").unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "a");
        assert_eq!(deps[0].version, Some("0.1.0".to_string()));

        let is_ready = env.check_pkg_ready("a#0.1.0");
        assert_eq!(is_ready.is_ok(), true);

        let media_info = env.load_strictly("a#*").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        let media_info = env.load_strictly("a#>0.0.1").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        let media_info = env.load_strictly("a#sha256:sha256xxxx").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));
    }

    #[test]
    fn test_try_load() {
        //创建一个临时目录
        let tmp_dir = tempdir().unwrap();
        let env = PackageEnv::new(tmp_dir.path().to_path_buf());
        //创建meta目录
        let meta_dir = env.get_meta_dir();
        fs::create_dir(&meta_dir).unwrap();

        //创建一个a#0.1.0的文件夹
        let pkg_dir = env.get_install_dir().join("a#0.1.0");
        fs::create_dir(&pkg_dir).unwrap();

        //测试try_load
        let media_info = env.load("a#0.1.0").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        let media_info = env.load("a#*").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        let media_info = env.load("a#>0.0.1").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("0.1.0".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a#0.1.0"));

        let ret = env.load("a#0.1.1");
        assert_eq!(ret.is_err(), true);

        let ret = env.load("a#>0.1.0");
        assert_eq!(ret.is_err(), true);

        let ret = env.load("a#>=0.1.0");
        assert_eq!(ret.is_ok(), true);
    }

    #[test]
    fn test_try_load_without_version() {
        //创建一个临时目录
        let tmp_dir = tempdir().unwrap();
        let env = PackageEnv::new(tmp_dir.path().to_path_buf());

        //创建一个a#0.1.0的文件夹
        let pkg_dir = env.get_install_dir().join("a");
        fs::create_dir(&pkg_dir).unwrap();

        let media_info = env.load("a#*").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("*".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a"));

        let media_info = env.load("a").unwrap();
        assert_eq!(media_info.pkg_id.name, "a");
        assert_eq!(media_info.pkg_id.version, Some("*".to_string()));
        assert_eq!(media_info.full_path, env.get_install_dir().join("a"));
    }
}
