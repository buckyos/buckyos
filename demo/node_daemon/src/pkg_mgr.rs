use std::path::PathBuf;
use regex::Regex;

/* 
pkg_id由两部分组成，包名和版本号或者sha256值。例如：
pkg_name
pkg_name#>0.1.4, pkg_name#>=0.1.4
pkg_name#0.1.5
pkg_name#sha256:1234567890
pkg_name#<0.1.6, pkg_name#<=0.1.6
pkg_name#>0.1.4<0.1.6, pkg_name#>0.1.4<=0.1.6, pkg_name#>=0.1.4<0.1.6, pkg_name#>=0.1.4<=0.1.6
 */
#[derive(Clone)]
pub struct PackageId {
    pub name: String,
    pub version: Option<String>,
    pub sha256: Option<String>,
}

/* 
PackageEnv是一个包管理的环境，下载，安装，加载都在某个env下面进行
一般来说，一个env对应一个工作目录
一个env包含：
index_db: 索引数据库，记录版本与sha256的对应关系
pkg.lock: 记录当前环境下已经安装的包精确信息

在包安装时，需要解析pkg_id，根据pkg_id从 index_db 中获取sha256值，然后下载
安装时，env第一级目录都是直接依赖的包，用npm举例来说，就是package.json中的dependencies，包内部依赖的包以及devDeps不会出现在这里，避免幽灵依赖
env第一级目录中是包的软连接 和 .pkgs 目录，.pkgs目录中才是包的真实内容
所有包的真实内容都在.pkgs的第一级目录中，第一级目录都是类似于 pkg_name#version 这样的文件夹，比如 core#v1.0.3，也就是可以同时依赖同名但版本不同的包
在安装时，会先检查是否已经安装了这个包，如果已经安装了，就不再重复安装，如果没有安装，就下载，解压到.pkgs的第一级目录，然后创建软连接
对于包内部的依赖，也都会提升到 .pkgs 的第一级目录中去查找，避免依赖地狱
所以理论上，.pkgs 下面的文件，最多只有1级子目录，第一级是实际的包目录，第二级是包内部依赖的软连接，而软链接又会链接到第一级目录中
这里要考虑的问题就是删除一个包时需要做的操作，理论上 .pkgs 下面的第一级目录只要没有任何软链接指向它，就可以删除，定时删除或者按需删除也可以是一个方案
还有在安装时如果安装失败是否要回退的问题。
 */
pub struct PackageEnv {
    //用来构建env的目录
    pub work_dir: PathBuf,
}

/* MediaInfo是一个包的元信息
   包括pkg_id，
   类型（dir or file）
   完整路径
 */
pub enum MediaType {
    Dir,
    File,
}

pub struct MediaInfo {
    pub pkg_id: PackageId,
    pub full_path: PathBuf,
    pub media_type: MediaType,
}

use thiserror::Error;
#[derive(Error, Debug)]
pub enum PackageSystemErrors {
    #[error("Download {0} error: {1}")]
    DownloadError(String, String),
    #[error("Load {0} error:{1}")]
    LoadError(String, String),
    #[error("Install {0} error: {1}")]
    InstallError(String, String),
    #[error("Parse {0} error")]
    ParseError(String),
}

type Result<T> = std::result::Result<T, PackageSystemErrors>;

impl PackageEnv {
    pub fn new(work_dir: PathBuf) -> Self {
        PackageEnv { work_dir }
    }

    pub fn get_work_dir(&self) -> &PathBuf {
        &self.work_dir
    }

    // load 一个包，从env根目录中查找目标pkg，找到了就返回一个MediaInfo结构，env文件结构见末尾
    pub async fn load_pkg(&self, pkg_id: &str) -> Result<MediaInfo> {
        let pkg_id = self.parse_pkg_id(pkg_id)?;
        //判断work_dir下是否存在pkg_id对应的目录
        let mut full_path = self.work_dir.join(&pkg_id.name);
        if let Some(version) = &pkg_id.version {
            //如果version不是以>=,<=,>,<开头，就是精确版本号
            if !version.starts_with(">") && !version.starts_with("<") {
                full_path.push(format!("#{}",  version));

                return self.load_with_full_path(&pkg_id, &full_path);
            }
        }

        //如果有精确的sha256值，也可以拼接
        if let Some(sha256) = &pkg_id.sha256 {
            full_path.push(format!("#{}",  sha256));

            if let Ok(media_info) = self.load_with_full_path(&pkg_id, &full_path) {
                return Ok(media_info);
            }
        }

        self.load_with_version_expression(&pkg_id)
    }

    // install 一个包，安装时一定要有确定的版本号或者sha256值
    pub async fn install_pkg(&self, pkg_id: &str) -> Result<()> {
        let mut pkg_id = self.parse_pkg_id(pkg_id)?;
        if self.load_pkg(&pkg_id.name).await.is_ok() {
            return Ok(());
        }

        // 如果sha256没有就查询，查询不到就失败
        if pkg_id.sha256.is_none() {
            if pkg_id.version.is_none() {
                return Err(PackageSystemErrors::InstallError(
                    pkg_id.name.clone(),
                    "No version or sha256 specified".to_string()
                ));
            }
            pkg_id.sha256 = self.get_sha256_from_version(&pkg_id.name, pkg_id.version.as_ref().unwrap())?;
        }

        let full_path = self.download_pkg(&pkg_id).await?;

        let install_path = self.get_install_path(&pkg_id)?;
        // TODO 解压到install_path
        let dep_file_path = install_path.join("deps.toml");
        let deps = self.get_deps(&pkg_id, &dep_file_path)?;

        for dep in deps {
            self.install_pkg(&dep).await?;
        }

        // TODO 创建软连接
    }

    fn get_deps(&self, pkg_id: &PackageId, dep_file_path: &PathBuf) -> Result<Vec<String>> {
        // TODO 根据dep_file_path描述获取依赖
        unimplemented!();
    }

    fn get_install_path(&self, pkg_id: &PackageId) -> Result<PathBuf> {
        // 如果有version，优先用version，否则用sha256
        let dest_dir = self.work_dir.join(".pkgs");
        if let Some(version) = &pkg_id.version {
            dest_dir.join(format!("{}#{}", pkg_id.name, version))
        } else if let Some(sha256) = &pkg_id.sha256 {
            dest_dir.join(format!("{}#{}", pkg_id.name, sha256))
        } else {
            Err(PackageSystemErrors::InstallError(
                pkg_id.name.clone(),
                "No version or sha256 specified".to_string()
            ))
        }
    }

    // download 一个包
    pub async fn download_pkg(&self, pkg_id: &PackageId) -> Result<PathBuf> {
        unimplemented!();
    }

    /* 解析pkg_id */
    pub fn parse_pkg_id(&self, pkg_id: &str) -> Result<PackageId> {
        let mut name = String::new();
        let mut version = None;
        let mut sha256 = None;

        let mut parts = pkg_id.split('#');
        if let Some(name_part) = parts.next() {
            name = name_part.to_string();
        } else {
            return Err(PackageSystemErrors::ParseError(
                pkg_id.to_string()
            ));
        }

        if let Some(version_part) = parts.next() {
            if version_part.starts_with("sha256:") {
                sha256 = Some(version_part[7..].to_string());
                version = self.get_version_from_sha256(&name, &sha256.as_ref().unwrap())?;
            } else {
                version = Some(version_part.to_string());
                // 这里先不做sha256的查询，等到下载时再查询
            }
        } else {
            version = self.get_default_version(&name)?;
        }

        Ok(PackageId { name, version, sha256 })
    }

    pub fn get_version_from_sha256(&self, pkg_name: &str, sha256: &str) -> Result<Option<String>> {
        // TODO 查询index_db
        unimplemented!();
    }

    pub fn get_sha256_from_version(&self, pkg_name: &str, version: &str) -> Result<Option<String>> {
        // TODO 查询index_db
        unimplemented!();
    }

    pub fn get_default_version(&self, pkg_name: &str) -> Result<Option<String>> {
        // TODO 查询package.lock中存在的版本
        // 或者查询index_db，默认获取index_db中最新的？
        unimplemented!();
    }

    fn load_with_full_path(&self, pkg_id: &PackageId, full_path: &PathBuf) -> Result<MediaInfo> {
        if full_path.exists() {
            let media_type = if full_path.is_dir() {
                MediaType::Dir
            } else {
                MediaType::File
            };

            Ok(MediaInfo {
                pkg_id: pkg_id.clone(),
                full_path: full_path.clone(),
                media_type,
            })
        } else {
            Err(PackageSystemErrors::LoadError(
                full_path.to_str().unwrap().to_string(),
                "not found".to_string()
            ))
        }
    }

    fn load_with_version_expression(&self, pkg_id: &PackageId) -> Result<MediaInfo> {
        let mut min_version = None;
        let mut max_version = None;
        let mut inclusive_min = false;
        let mut inclusive_max = false;

        if let Some(version) = &pkg_id.version {
            if !version.starts_with(">") && !version.starts_with("<") {
                return Err(PackageSystemErrors::LoadError(
                    pkg_id.name.clone(),
                    "Invalid version expression".to_string()
                ));
            }

            // 使用正则表达式来匹配版本号和操作符， 一般是类似>1.0.2 或者 >1.0.2<1.0.5这样的版本表达式
            let re = Regex::new(r"(>=|<=|>|<)(\d+\.\d+\.\d+)").unwrap();

            for cap in re.captures_iter(version) {
                match &cap[1] {
                    ">=" => {
                        min_version = Some(cap[2].to_string());
                        inclusive_min = true;
                    },
                    ">" => {
                        min_version = Some(cap[2].to_string());
                    },
                    "<=" => {
                        max_version = Some(cap[2].to_string());
                        inclusive_max = true;
                    },
                    "<" => {
                        max_version = Some(cap[2].to_string());
                    },
                    _ => {}
                }
            }
        }

        // 找到符合条件的版本
        let matching_version = self.find_matching_version(&pkg_id.name, min_version, max_version, inclusive_min, inclusive_max)?;
    
        let full_path = self.work_dir.join(format!("{}#{}", pkg_id.name, matching_version));
    
        self.load_with_full_path(&pkg_id, &full_path)
    }
    
    fn find_matching_version(
        &self,
        pkg_name: &str,
        min_version: Option<String>,
        max_version: Option<String>,
        inclusive_min: bool,
        inclusive_max: bool,
    ) -> Result<String> {
        //这里可以查询pkg.lock或者直接遍历目录？
        unimplemented!();
    }
}
