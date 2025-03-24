/*

pkg-env的目录结构设计
在work-dir下有一系列json格式的配置文件
pkg.cfg.json envd的配置文件，不存在则env使用默认配置
.pkgs/env.lock 锁文件，保证多进程的情况下只有一个进程可以进行写操作
.pkgs/meta_index.db 元数据索引数据库
.pkgs/meta_index.db.old 元数据索引数据库的备份文件
.pkgs/pkg_nameA/$pkg_id pkg的实体安装目录
pkg_nameA --> pkg_nameA的默认版本 链接到.pkgs/pkg_nameA$pkg_id目录
pkg_nameA#1.0.3 --> pkg_nameA的已安装版本 链接到.pkgs/pkg_nameA$pkg_id目录

# pkg_id有两种格式
- 语义pkg_id: pkg_nameA#1.0.3
- 准确pkg_id: pkg_nameA$meta_obj_id ,ob_id的写法是objtype:objid ,比如sha256:1234567890 就是一个合法的meta_obj_id
- 也允许  pkg_nameA#1.0.3#meta_obj_id 这样的写法相当于准确pkg_id

下面是python的伪代码，注意正式的实现都是async的

#根据pkg_id加载已经成功安装的pkg
def env.load(pkg_id):
    meta_db = get_meta_db()
    if meta_db
        pkg_meta = meta_db.get_pkg_meta(pkg_id)
        if pkg_meta:
            #得到.pkgs/pkg_nameA/$pkg_id目录
            pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
            if os.exist(pkg_strict_dir)
                return PkgMediaInfo(pkg_strict_dir)
        
    if not self.strict_mode:
        pkg_dir = get_pkg_dir(pkg_id)
        if pkg_dir:
            pkg_meta_file = pkg_dir.append(".pkg.meta")
            local_meta = load_meta_file(pkg_meta_file)
            if local_meta:
                if staticfy(local_meta.version,pkg_id):
                    return PkgMediaInfo(pkg_dir)
            else:
                return return PkgMediaInfo(pkg_dir)


#根据pkg_id加载pkg_meta
def env.get_pkg_meta(pkg_id):
    if self.lock_db:
        lock_meta = self.lock_db.get(pkg_id)
        if lock_meta:
            return lock_meta

    if meta_db
        pkg_meta = meta_db.get_pkg_meta(pkg_id)
        if pkg_meta:
            pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
            if os.exist(pkg_strict_dir)
                return PkgMediaInfo(pkg_strict_dir)

#根据pkg_id判断是否已经成功安装，注意会对deps进行检查
def env.check_pkg_ready(pkg_id,need_check_deps):
    pkg_meta = get_pkg_meta(pkg_id)
    if pkg_meta is none:
        return false

    if pkg_meta.chunk_id:
        if not ndn_mgr.is_chunk_exist(pkg_meta.chunk_id):
            return false;

    if need_check_deps:
        deps = env.cacl_pkg_deps(pkg_meta)
        for pkg_id in deps:
            if not check_pkg_ready(pkg_id,false):
                return false
        
        return true
# 在env中安装pkg
def env.install_pkg(pkg_id,install_deps)
    env.lock_for_write() #注意这是一个写操作，要做基于文件系统的全局锁

    if self.ready_only
        return err("READ_ONLY")

    pkg_meta = get_pkg_meta(pkg_id)
    if pkg_meta is none:
        return err("unknown pkg_id")
    if install_deps:
        deps = env.cacl_pkg_deps(pkg_meta)

    //有一个消费者线程专门处理单个pkg的安装
    task_id,is_new_task = env.install_task.insert(pkg_id)
    if is_new_task && install_deps:
        for pkg_id in deps:
            env.install_task.insert_sub_task(pkg_id,task_id)
        
    return task_id,is_new_task

# 内部函数，从install_task队列中提取任务执行
def env.install_worker():
    let install_task = env.install_task.pop()
    #下载到env配置的临时目录，不配置则下载到ndn_mgr的统一chunk目录
    download_result = download_chunk(install_task.pkg_meta.chunkid)
    if download_result:
        pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
        unzip(download_result.fullpath,pkg_strict_dir)
        if self.enable_link:
            create_link(install_task.pkg_meta)
        notify_task_done(install_task)

# 异步编程支持,可以等待一个task的结束
def env.wait_task(taskid)

# 尝试更新env的meta-index-db,传入的new_index_db是新的index_db的本地路径
def env.try_update_index_db(new_index_db):
    #当有安装任务存在时，无法更新index_db
    env.try_lock_for_write()
    #重命名当前文件
    rename_file(index_db,index_db.append(".old"))
    #移动新文件到当前目录
    rename_file(new_index_db,index_db)
    #删除旧版本的数据库文件
    delete_file(index_db.append(".old"))

*/

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use async_trait::async_trait;
use log::*;

//use std::fs::File;
//use std::io;
use tokio::io::AsyncReadExt;
use tokio::fs::File;
use async_compression::tokio::bufread::GzipDecoder;
use tokio::io::BufReader;
use tokio_tar::Archive;
use async_fd_lock::{LockRead, LockWrite};
use async_fd_lock::RwLockWriteGuard;
use ndn_lib::*;

use crate::error::*;
use crate::meta::*;
use crate::package_id::*;
use crate::meta_index_db::*;



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEnvConfig {
    pub enable_link: bool,
    pub enable_strict_mode: bool,
    pub index_db_path: Option<String>,
    pub parent: Option<PathBuf>, //parent package env work_dir
    pub ready_only: bool,
    pub named_mgr_name: Option<String>, //如果指定了，则使用named_mgr_name作为命名空间
    pub prefix: Option<String>, //如果指定了，那么加载无 .符号的pkg_name时，会自动补上prefix
}

impl PackageEnvConfig {
    pub fn get_default_prefix() -> String {
        //得到操作系统类型
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let os_type = "nightly-linux-x86_64";
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        let os_type = "nightly-linux-aarch64";
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        let os_type = "nightly-windows-x86_64";
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let os_type = "nightly-apple-x86_64";
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let os_type = "nightly-apple-aarch64";

        os_type.to_string()
    }
}

impl Default for PackageEnvConfig {
    fn default() -> Self {
        let os_type = PackageEnvConfig::get_default_prefix();

        Self {
            enable_link: true,
            enable_strict_mode: false, //默认是非严格的开发模式
            index_db_path: None,
            parent: None,   
            ready_only: false,
            named_mgr_name: None,
            prefix: Some(os_type.to_string()),
        }
    }
}

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

#[derive(Clone)]
pub struct PackageEnv {
    pub work_dir: PathBuf,
    pub config: PackageEnvConfig,
    lock_db: Arc<TokioMutex<Option<HashMap<String, (String,PackageMeta)>>>>,
    install_tasks: Arc<TokioMutex<HashMap<String, InstallTask>>>,
    task_notifiers: Arc<TokioMutex<HashMap<String, oneshot::Receiver<()>>>>,
}

#[derive(Debug)]
struct InstallTask {
    pkg_id: String,
    status: InstallStatus,
    sub_tasks: Vec<String>,
}

#[derive(Debug)]
enum InstallStatus {
    Pending,
    Downloading,
    Installing,
    Completed,
    Failed(String),
}

impl PackageEnv {
    pub fn new(work_dir: PathBuf) -> Self {
        let config_path = work_dir.join("pkg.cfg.json");
        let mut env_config = PackageEnvConfig::default();
        if config_path.exists() {
            let config = std::fs::read_to_string(config_path);
            if config.is_ok() {
                let config_result = serde_json::from_str(&config.unwrap());
                if  config_result.is_ok() {
                    env_config = config_result.unwrap();
                }
            }
        }
        
        Self {
            work_dir,
            config: env_config,
            lock_db: Arc::new(TokioMutex::new(None)),
            install_tasks: Arc::new(TokioMutex::new(HashMap::new())),
            task_notifiers: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    // 基于env获得pkg的meta信息
    pub async fn get_pkg_meta(&self, pkg_id: &str) -> PkgResult<(String,PackageMeta)> {
        // 先检查lock db
        if let Some(lock_db) = self.lock_db.lock().await.as_ref() {
            if let Some((meta_obj_id,meta)) = lock_db.get(pkg_id) {
                return Ok((meta_obj_id.clone(),meta.clone()));
            }
        }

        let meta_db_path = self.get_meta_db_path();
        let meta_db = MetaIndexDb::new(meta_db_path,true)?;
        if let Some((meta_obj_id,pkg_meta)) = meta_db.get_pkg_meta(pkg_id)? {
             return Ok((meta_obj_id,pkg_meta));
        }

        if self.config.parent.is_some() {
            let parent_env = PackageEnv::new(self.config.parent.as_ref().unwrap().clone());
            let (meta_obj_id,pkg_meta) = Box::pin(parent_env.get_pkg_meta(pkg_id)).await?;
            return Ok((meta_obj_id,pkg_meta));
        }
        
        Err(PkgError::LoadError(
            pkg_id.to_owned(),
            "Package metadata not found".to_owned(),
        ))
    }

    //加载pkg,加载成功说明pkg已经安装
    pub async fn load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        match self.load_strictly(pkg_id_str).await {
            Ok(media_info) => Ok(media_info),
            Err(_) => {
                if self.config.enable_strict_mode {
                    if let Some(parent_path) = &self.config.parent {
                        let parent_env = PackageEnv::new(parent_path.clone());
                        // 使用 Box::pin 来处理递归的异步调用
                        let future = Box::pin(parent_env.load(pkg_id_str));
                        if let Ok(media_info) = future.await {
                            return Ok(media_info);
                        }
                    }
                } else {
                    info!("dev mode env {} : try load pkg: {}", self.work_dir.display(), pkg_id_str);
                    let media_info = self.dev_try_load(pkg_id_str).await;
                    if media_info.is_ok() {
                        return Ok(media_info.unwrap());
                    }
                    if let Some(parent_path) = &self.config.parent {
                        let parent_env = PackageEnv::new(parent_path.clone());
                        let future = Box::pin(parent_env.load(pkg_id_str));
                        if let Ok(media_info) = future.await {
                            return Ok(media_info);
                        }
                    }
                }

                info!("load pkg {} failed.", pkg_id_str);
                Err(PkgError::LoadError(
                    pkg_id_str.to_owned(),
                    "Package metadata not found".to_owned(),
                ))
            }
        }
    }

    //检查pkg的依赖是否都已经在本机就绪，注意本操作并不会修改env
    pub async fn check_pkg_ready(meta_index_db: PathBuf, pkg_id: &str, named_mgr_id: Option<&str>, need_check_deps: bool) -> PkgResult<()> {
        let meta_db = MetaIndexDb::new(meta_index_db.clone(),true)?;
        let meta_info = meta_db.get_pkg_meta(pkg_id)?;
        if meta_info.is_none() {
            return Err(PkgError::LoadError(
                pkg_id.to_owned(),
                "Package metadata not found".to_owned(),
            ));
        }

        let (meta_obj_id,pkg_meta) = meta_info.unwrap();
        // 检查chunk是否存在
        if let Some(chunk_id) = pkg_meta.chunk_id {
            // TODO: 实现chunk存在性检查
            let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(named_mgr_id).await;
            if named_mgr.is_none() {
                return Err(PkgError::FileNotFoundError(
                    "Named data mgr not found".to_owned(),
                ));
            }
            let named_mgr = named_mgr.unwrap();
            let named_mgr = named_mgr.lock().await;
            let chunk_id = ChunkId::new(&chunk_id)
                .map_err(|e| PkgError::ParseError(
                    pkg_id.to_owned(),
                    format!("Invalid chunk id: {}", e),
                ))?;

            let is_chunk_exist = named_mgr.is_chunk_exist_impl(&chunk_id).await
                .map_err(|e| PkgError::ParseError(
                    pkg_id.to_owned(),
                    format!("Chunk not found: {}", e),
                ))?;

            if !is_chunk_exist {
                return Err(PkgError::InstallError(
                    pkg_id.to_owned(),
                    "Chunk not found".to_owned(),
                ));
            }
        }

        if need_check_deps {
            for (dep_name, dep_version) in pkg_meta.deps.iter() {
                let dep_id = format!("{}#{}", dep_name, dep_version);
                let check_future = Box::pin(PackageEnv::check_pkg_ready(meta_index_db.clone(), &dep_id, named_mgr_id, true));
                let _ = check_future.await?;
            }
        }

        Ok(())
    }

    //尝试更新env的meta-index-db,这是个写入操作，更新后之前的load操作可能会失败，需要再执行一次install_pkg才能加载
    pub async fn try_update_index_db(&self, new_index_db: &Path) -> PkgResult<()> {
        if self.config.ready_only {
            return Err(PkgError::AccessDeniedError(
                "Cannot update index db in read-only mode".to_owned(),
            ));
        }

        let _lock = self.acquire_lock().await?;

        let mut index_db_path = self.get_meta_db_path();    
        let backup_path = index_db_path.with_extension("old");
        if tokio_fs::metadata(&backup_path).await.is_ok() {
            tokio_fs::remove_file(&backup_path).await?;
            info!("delete backup index db: {:?}", backup_path);
        }

        if tokio_fs::metadata(&index_db_path).await.is_ok() {
            let backup_path = index_db_path.with_extension("old");
            info!("rename old index db: {:?} to {:?}", index_db_path, backup_path);
            tokio_fs::rename(&index_db_path, &backup_path).await?;
        }

        // 移动新数据库
        tokio_fs::copy(new_index_db, &index_db_path).await?;
        info!("update index db: {:?} OK", index_db_path);
        Ok(())
    }

    //安装pkg，安装成功后该pkg可以加载成功,返回安装成功的pkg的meta_obj_id
    //安装操作会锁定env，直到安装完成（不会出现两个安装操作同时进行）
    //安装过程会根据env是否支持符号链接，尝试建立有好的符号链接
    //在parent envinstall pkg成功，会对所有的child env都有影响
    //在child env install pkg成功，对parent env没有影响
    pub async fn install_pkg(&self, pkg_id: &str, install_deps: bool) -> PkgResult<String> {
        if self.config.ready_only {
            return Err(PkgError::InstallError(
                pkg_id.to_owned(),
                "Cannot install in read-only mode".to_owned(),
            ));
        }

        if install_deps {
            return Err(PkgError::InstallError(
                pkg_id.to_owned(),
                "Install deps is not supported at this version".to_owned(),
            ));
        }

        // 获取文件锁
        let _filelock = self.acquire_lock().await?;
        //先将必要的chunk下载到named_mgr中,对于单OOD系统，这些chunk可能都已经准备好了
        let (meta_obj_id,pkg_meta) = self.get_pkg_meta(pkg_id).await?;

        //检查chunk是否存在
        if let Some(ref chunk_id_str) = pkg_meta.chunk_id {

            let chunk_id = ChunkId::new(&chunk_id_str)
                .map_err(|e| PkgError::ParseError(
                    pkg_id.to_owned(),
                    format!("Invalid chunk id: {}", e),
                ))?;
            if !NamedDataMgr::have_chunk(&chunk_id,self.config.named_mgr_name.as_deref()).await {
                info!("{}'s chunk {} not found, downloading...", pkg_id, chunk_id_str);
                let zone_repo_url = "http://127.0.0.1:8080/repo";
                let ndn_client = NdnClient::new(zone_repo_url.to_string(),None,self.config.named_mgr_name.clone());
                let chunk_size = ndn_client.pull_chunk(chunk_id.clone(),None).await
                    .map_err(|e| PkgError::DownloadError(
                        pkg_id.to_owned(),
                        format!("Failed to download chunk: {}", e),
                    ))?;
                info!("chunk {} downloaded, size: {}", chunk_id_str, chunk_size);

            }

            let (chunk_reader,chunk_size) = NamedDataMgr::open_chunk_reader(self.config.named_mgr_name.as_deref(),
                &chunk_id,SeekFrom::Start(0),false).await
                .map_err(|e| PkgError::LoadError(
                    pkg_id.to_owned(),
                    format!("Failed to open chunk reader: {}", e),
                ))?;

            self.extract_pkg_from_chunk(&pkg_meta,meta_obj_id.as_str(),chunk_reader).await?;
            info!("{} extract chunk to pkg_env OK.", pkg_id);
        }

        Ok(meta_obj_id)   
    }

    async fn extract_pkg_from_chunk(&self, pkg_meta: &PackageMeta,meta_obj_id: &str,chunk_reader: ChunkReader) -> PkgResult<()> {
        //将chunk (这是一个tar.gz文件)解压安装到真实目录 .pkgs/pkg_nameA/$meta_obj_id
        //注意处理前缀: 如果包名与当前env前缀相同，那么符号链接里只包含无前缀部分
        //建立符号链接 ./pkg_nameA#version -> .pkgs/pkg_nameA/$meta_obj_id
        //如果是最新版本，建立符号链接 ./pkg_nameA -> .pkgs/pkg_nameA/$meta_obj_id
    

        // Decompress the tar.gz file
        let buf_reader = BufReader::new(chunk_reader);
        // 创建异步 GZip 解压器
        let gz_decoder = GzipDecoder::new(buf_reader);
        // 创建异步 tar 解压器
        let mut archive = Archive::new(gz_decoder);
        let target_dir = format!(".pkgs/{}/{}", pkg_meta.pkg_name, meta_obj_id);
        tokio::fs::create_dir_all(&target_dir).await?;
        // 解压文件到目标目录
        archive.unpack(&target_dir).await?;

        // Create symbolic links
        if self.config.enable_link {
            let symlink_path = format!("./{}#{}", pkg_meta.pkg_name, pkg_meta.version);
            if tokio::fs::symlink_metadata(&symlink_path).await.is_err() {
                #[cfg(target_family = "unix")]
                tokio::fs::symlink(&target_dir, &symlink_path).await?;
                #[cfg(target_family = "windows")]
                std::os::windows::fs::symlink_dir(&target_dir, &symlink_path)?;
            }
        
            // If this is the latest version, create a symbolic link without the version
            // if self.is_latest_version(&pkg_meta.pkg_name, &pkg_meta.version).await? {
            //     let latest_symlink_path = format!("./{}", pkg_meta.pkg_name);
            //     if tokio::fs::symlink_metadata(&latest_symlink_path).await.is_err() {
            //         tokio::fs::symlink(&target_dir, &latest_symlink_path).await?;
            //     }
            // }
        }

        Ok(())
    }

    fn get_prefix(&self) -> String {
        if let Some(prefix) = &self.config.prefix {
            prefix.clone()
        } else {
            PackageEnvConfig::get_default_prefix()
        }
    }

    async fn load_strictly(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        let mut pkg_id = PackageId::parse(pkg_id_str)?;
        if pkg_id.name.find(".").is_none() {
            let real_pkg_id = format!("{}.{}", self.get_prefix(), pkg_id.name.as_str());
            pkg_id.name = real_pkg_id;
        }
        // 在严格模式下，先获取包的元数据以获得准确的物理目录
        let real_pkg_id = pkg_id.to_string();
        let (meta_obj_id,pkg_meta) = self.get_pkg_meta(&real_pkg_id).await?;
        
        // 使用元数据中的信息构建准确的物理路径
        let pkg_strict_dir = self.get_pkg_strict_dir(&meta_obj_id,&pkg_meta);
        
        if tokio_fs::metadata(&pkg_strict_dir).await.is_ok() {
            let metadata = tokio_fs::metadata(&pkg_strict_dir).await?;
            let media_type = if metadata.is_dir() {
                MediaType::Dir
            } else {
                MediaType::File
            };
            
            return Ok(MediaInfo {
                pkg_id,
                full_path: pkg_strict_dir,
                media_type,
            });
        }
        
        Err(PkgError::LoadError(
            pkg_id_str.to_owned(),
            "包在严格模式下未找到".to_owned(),
        ))
    }


    async fn dev_try_load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        let pkg_dirs = self.get_pkg_dir(pkg_id_str)?;
        for pkg_dir in pkg_dirs {
            debug!("try load pkg {} from {}", pkg_id_str, pkg_dir.display());
            if tokio_fs::metadata(&pkg_dir).await.is_ok() {
                debug!("try load pkg {} from {} OK.", pkg_id_str, pkg_dir.display());
                return Ok(MediaInfo {
                    pkg_id: PackageId::parse(pkg_id_str)?,
                    full_path: pkg_dir,
                    media_type: MediaType::Dir,
                });
            }
        }
        Err(PkgError::LoadError(
            pkg_id_str.to_owned(),
            "Package not found".to_owned(),
        ))
    }


    fn get_install_dir(&self) -> PathBuf {
        self.work_dir.join(".pkgs")
    }

    fn get_meta_db_path(&self) -> PathBuf {
        let mut meta_db_path;
        if let Some(index_db_path) = &self.config.index_db_path {
            meta_db_path = PathBuf::from(index_db_path);
        } else {
            meta_db_path = self.work_dir.join(".pkgs/meta_index.db")
        }
        meta_db_path
    }


    fn get_pkg_strict_dir(&self, meta_obj_id: &str,pkg_meta: &PackageMeta) -> PathBuf {
        let pkg_name = pkg_meta.pkg_name.clone();
        //.pkgs/pkg_nameA/$meta_obj_id
        self.get_install_dir().join(pkg_name).join(meta_obj_id)
    }

    fn get_pkg_dir(&self, pkg_id: &str) -> PkgResult<Vec<PathBuf>> {
        let pkg_id = PackageId::parse(pkg_id)?;
        let pkg_name = pkg_id.name.clone();
        let mut pkg_dirs = Vec::new();
        
        if pkg_id.objid.is_some() {
            pkg_dirs.push(self.get_install_dir().join(".pkgs").join(pkg_name).join(pkg_id.objid.unwrap()));
        } else {
            if pkg_id.version_exp.is_some() {
               //TODO: 要考虑如何结合lock文件进行查找
               pkg_dirs.push(self.get_install_dir().join(pkg_name));
            } else {
                pkg_dirs.push(self.work_dir.join(pkg_name));
            }
        }
        Ok(pkg_dirs)
    }


    
    // 添加一个新的私有方法来管理锁文件
    async fn acquire_lock(&self) -> PkgResult<RwLockWriteGuard<File>> {
        let lock_path = self.work_dir.join(".pkgs/env.lock");
        
        // 确保.pkgs目录存在
        if let Err(e) = tokio_fs::create_dir_all(self.work_dir.join(".pkgs")).await {
            return Err(PkgError::LockError(format!("Failed to create lock directory: {}", e)));
        }

        // 以读写模式打开或创建锁文件
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path).await
            .map_err(|e| PkgError::LockError(format!("Failed to open lock file: {}", e)))?;
        
        let lock = file.lock_write().await
            .map_err(|e| PkgError::LockError(format!("Failed to open lock file: {:?}", lock_path)))?;
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    async fn setup_test_env() -> (PackageEnv, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let env = PackageEnv::new(temp_dir.path().to_path_buf());
        
        // 创建.pkgs目录
        tokio_fs::create_dir_all(env.get_install_dir()).await.unwrap();
        
        (env, temp_dir)
    }

    async fn create_test_package(env: &PackageEnv, pkg_name: &str, version: &str) -> PathBuf {
        let pkg_dir = env.get_install_dir().join(format!("{}#{}", pkg_name, version));
        tokio_fs::create_dir_all(&pkg_dir).await.unwrap();
        
        // 创建meta文件
        let meta = PackageMeta {
            pkg_name: pkg_name.to_string(),
            description:json!({}),
            version: version.to_string(),
            tag: Some("test".to_string()),
            category: Some("test".to_string()),
            author: "test".to_string(),
            chunk_id: Some("test_chunk".to_string()),
            chunk_size: Some(100),
            chunk_url: Some("http://test.com".to_string()),
            deps: HashMap::new(),
            pub_time: 0,
            exp: 0,
            extra_info:HashMap::new()
        };
        
        let meta_path = pkg_dir.join(".pkg.meta");
        tokio_fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).await.unwrap();
        
        pkg_dir
    }

    #[tokio::test]
    async fn test_load_strictly() {
        let (env, _temp) = setup_test_env().await;
        
        // 创建测试包
        let pkg_dir = create_test_package(&env, "test-pkg", "1.0.0").await;
        
        // 测试严格模式加载
        let media_info = env.load_strictly("test-pkg#1.0.0").await.unwrap();
        assert_eq!(media_info.pkg_id.name, "test-pkg");
        assert_eq!(media_info.pkg_id.version_exp.as_ref().unwrap().to_string(), "1.0.0".to_string());
        assert_eq!(media_info.full_path, pkg_dir);
        
        // 测试不存在的包
        assert!(env.load_strictly("not-exist#1.0.0").await.is_err());
    }

    #[tokio::test]
    async fn test_try_load() {
        let (env, _temp) = setup_test_env().await;
        
        // 创建测试包
        let pkg_dir = create_test_package(&env, "test-pkg", "1.0.0").await;
        
        // 测试模糊版本匹配
        let media_info = env.dev_try_load("test-pkg#*").await.unwrap();
        assert_eq!(media_info.pkg_id.name, "test-pkg");
        assert_eq!(media_info.full_path, pkg_dir);
        
        // 测试精确版本匹配
        let media_info = env.dev_try_load("test-pkg#1.0.0").await.unwrap();
        assert_eq!(media_info.pkg_id.name, "test-pkg");
        assert_eq!(media_info.pkg_id.version_exp.as_ref().unwrap().to_string(), "1.0.0".to_string());
        
        // 测试不存在的包
        assert!(env.dev_try_load("not-exist#1.0.0").await.is_err());
    }

    // #[tokio::test]
    // async fn test_install_pkg() {
    //     let (env, _temp) = setup_test_env().await;
        
    //     // 创建测试包及其依赖
    //     create_test_package(&env, "test-pkg", "1.0.0").await;
    //     create_test_package(&env, "dep-pkg", "0.1.0").await;
        
    //     // 测试安装包(不包含依赖)
    //     let task_id = env.install_pkg("test-pkg#1.0.0", false).await.unwrap();
    //     assert_eq!(task_id, "test-pkg#1.0.0");
        
    //     // 等待任务完成
    //     env.wait_task(&task_id).await.unwrap();
        
    //     // 验证任务状态
    //     let tasks = env.install_tasks.lock().await;
    //     let task = tasks.get(&task_id).unwrap();
    //     assert!(matches!(task.status, InstallStatus::Completed));
    //     assert!(task.sub_tasks.is_empty());
    // }

    #[tokio::test]
    async fn test_get_pkg_meta() {
        let (env, _temp) = setup_test_env().await;
        
        // 创建测试包
        create_test_package(&env, "test-pkg", "1.0.0").await;
        
        // 测试获取meta信息
        let (meta_obj_id,meta) = env.get_pkg_meta("test-pkg#1.0.0").await.unwrap();
        assert_eq!(meta.pkg_name, "test-pkg");
        assert_eq!(meta.version, "1.0.0".to_string());
        assert_eq!(meta.category, Some("test".to_string()));
        
        // 测试不存在的包
        assert!(env.get_pkg_meta("not-exist#1.0.0").await.is_err());
    }

 

    #[tokio::test]
    async fn test_try_update_index_db() {
        let (env, temp) = setup_test_env().await;
        
        // 创建测试数据库文件
        let new_db_path = temp.path().join("new_index.db");
        tokio_fs::write(&new_db_path, "test data").await.unwrap();
        
        // 测试更新数据库
        env.try_update_index_db(&new_db_path).await.unwrap();
        
        // 验证更新结果
        let db_path = env.work_dir.join(".pkgs/meta_index.db");
        assert!(tokio_fs::metadata(&db_path).await.is_ok());
        assert_eq!(tokio_fs::read_to_string(db_path).await.unwrap(), "test data");
    }
}
