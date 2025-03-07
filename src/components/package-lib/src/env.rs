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
use std::fs;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use async_trait::async_trait;
use log::*;

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
    pub parent: Option<String>, //parent package env work_dir
    pub ready_only: bool,
    pub named_mgr_name: Option<String>, //如果指定了，则使用named_mgr_name作为命名空间
}

impl Default for PackageEnvConfig {
    fn default() -> Self {
        Self {
            enable_link: true,
            enable_strict_mode: false, //默认是非严格的开发模式
            index_db_path: None,
            parent: None,   
            ready_only: false,
            named_mgr_name: None,
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

    // 获取pkg的meta信息
    pub async fn get_pkg_meta(&self, pkg_id: &str) -> PkgResult<(String,PackageMeta)> {
        // 先检查lock db
        if let Some(lock_db) = self.lock_db.lock().await.as_ref() {
            if let Some((meta_obj_id,meta)) = lock_db.get(pkg_id) {
                return Ok((meta_obj_id.clone(),meta.clone()));
            }
        }

        let meta_db = self.get_meta_db().await?;
        if let Some((meta_obj_id,pkg_meta)) = meta_db.get_pkg_meta(pkg_id)? {
            let pkg_strict_dir = self.get_pkg_strict_dir(&meta_obj_id,&pkg_meta);
            if tokio_fs::metadata(&pkg_strict_dir).await.is_ok() {
                return Ok((meta_obj_id,pkg_meta));
            }
        }

        //非严格模式下，用try_load加载后，返回目录下的meta_info

        Err(PkgError::LoadError(
            pkg_id.to_owned(),
            "Package metadata not found".to_owned(),
        ))
    }

    pub async fn load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        match self.load_strictly(pkg_id_str).await {
            Ok(media_info) => Ok(media_info),
            Err(_) => {
                if self.config.enable_strict_mode {
                    return Err(PkgError::LoadError(
                        pkg_id_str.to_owned(),
                        "env not found in strict mode".to_owned(),
                    ))
                }
                info!("dev mode pkg_env : try load pkg: {}", pkg_id_str);
                self.try_load(pkg_id_str).await
            }
        }
    }

    async fn load_strictly(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        let pkg_id = PackageId::parse(pkg_id_str)?;
        
        // 在严格模式下，先获取包的元数据以获得准确的物理目录
        let (meta_obj_id,pkg_meta) = self.get_pkg_meta(pkg_id_str).await?;
        
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

    async fn try_load(&self, pkg_id_str: &str) -> PkgResult<MediaInfo> {
        let pkg_dirs = self.get_pkg_dir(pkg_id_str)?;
        for pkg_dir in pkg_dirs {
            if tokio_fs::metadata(&pkg_dir).await.is_ok() {
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
    //是否有必要将install_pkg移动到另一个实现中，env中只包含支持install的基础设施
    pub async fn install_pkg(&self, pkg_id: &str, install_deps: bool) -> PkgResult<String> {
        if self.config.ready_only {
            return Err(PkgError::InstallError(
                pkg_id.to_owned(),
                "Cannot install in read-only mode".to_owned(),
            ));
        }

        let mut tasks = self.install_tasks.lock().await;
        let task_id = pkg_id.to_owned();

        if tasks.contains_key(&task_id) {
            return Ok(task_id);
        }

        let (meta_obj_id,pkg_meta) = self.get_pkg_meta(pkg_id).await?;
        let mut task = InstallTask {
            pkg_id: pkg_id.to_owned(),
            status: InstallStatus::Pending,
            sub_tasks: Vec::new(),
        };

        if install_deps {
            for (dep_name, dep_version) in pkg_meta.deps.iter() {
                let dep_id = format!("{}#{}", dep_name, dep_version);
                task.sub_tasks.push(dep_id);
            }
        }

        // 创建通知通道
        let (tx, rx) = oneshot::channel();
        self.task_notifiers.lock().await.insert(task_id.clone(), rx);

        tasks.insert(task_id.clone(), task);
        drop(tasks);  // 提前释放锁

        // 启动安装工作线程
        let install_tasks = self.install_tasks.clone();
        let task_id_clone = task_id.clone();
        
        tokio::spawn(async move {
            let mut tasks = install_tasks.lock().await;
            if let Some(task) = tasks.get_mut(&task_id_clone) {
                task.status = InstallStatus::Downloading;
                
                // TODO: 实现下载和安装逻辑
                // 1. 下载chunk
                // 2. 解压到目标目录
                // 3. 创建符号链接
                
                task.status = InstallStatus::Completed;
                
                // 通知任务完成
                let _ = tx.send(());
            }
        });

        Ok(task_id)
    }

    pub async fn wait_task(&self, task_id: &str) -> PkgResult<()> {
        if let Some(rx) = self.task_notifiers.lock().await.remove(task_id) {
            rx.await.map_err(|e| PkgError::InstallError(
                task_id.to_owned(),
                format!("Wait task error: {}", e),
            ))?;
        }
        Ok(())
    }

    fn get_install_dir(&self) -> PathBuf {
        self.work_dir.join(".pkgs")
    }

    async fn get_meta_db(&self) -> PkgResult<MetaIndexDb> {
        let mut meta_db_path;
        if let Some(index_db_path) = &self.config.index_db_path {
            meta_db_path = PathBuf::from(index_db_path);
        } else {
            meta_db_path = self.work_dir.join(".pkgs/meta_index.db")
        }
        let meta_db = MetaIndexDb::new(meta_db_path)?;
        Ok(meta_db)
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

    

    pub async fn check_pkg_ready(&self, pkg_id: &str, need_check_deps: bool) -> PkgResult<()> {
        let (meta_obj_id,pkg_meta) = self.get_pkg_meta(pkg_id).await?;
        
        // 检查chunk是否存在
        if let Some(chunk_id) = pkg_meta.chunk_id {
            // TODO: 实现chunk存在性检查
            let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(self.config.named_mgr_name.as_deref()).await;
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

            let is_chunk_exist = named_mgr.is_chunk_exist(&chunk_id).await
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
                let check_future = Box::pin(self.check_pkg_ready(&dep_id, true));
                let _ = check_future.await?;
            }
        }

        Ok(())
    }

    pub async fn try_update_index_db(&self, new_index_db: &Path) -> PkgResult<()> {
        if self.config.ready_only {
            return Err(PkgError::AccessDeniedError(
                "Cannot update index db in read-only mode".to_owned(),
            ));
        }

        let mut index_db_path;
        if let Some(index_db_path_str) = &self.config.index_db_path {
            index_db_path = PathBuf::from(index_db_path_str);
        } else {
            index_db_path = self.work_dir.join(".pkgs/meta_index.db");
        }
        
        let backup_path = index_db_path.with_extension("old");
        if tokio_fs::metadata(&backup_path).await.is_ok() {
            tokio_fs::remove_file(&backup_path).await?;
            info!("delete backup index db: {:?}", backup_path);
        }

        if tokio_fs::metadata(&index_db_path).await.is_ok() {
            let backup_path = index_db_path.with_extension("old");
            tokio_fs::rename(&index_db_path, &backup_path).await?;
        }

        // 移动新数据库
        tokio_fs::copy(new_index_db, &index_db_path).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            version: version.to_string(),
            tag: Some("test".to_string()),
            category: Some("test".to_string()),
            author: "test".to_string(),
            chunk_id: Some("test_chunk".to_string()),
            chunk_url: Some("http://test.com".to_string()),
            deps: HashMap::new(),
            pub_time: 0,
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
        let media_info = env.try_load("test-pkg#*").await.unwrap();
        assert_eq!(media_info.pkg_id.name, "test-pkg");
        assert_eq!(media_info.full_path, pkg_dir);
        
        // 测试精确版本匹配
        let media_info = env.try_load("test-pkg#1.0.0").await.unwrap();
        assert_eq!(media_info.pkg_id.name, "test-pkg");
        assert_eq!(media_info.pkg_id.version_exp.as_ref().unwrap().to_string(), "1.0.0".to_string());
        
        // 测试不存在的包
        assert!(env.try_load("not-exist#1.0.0").await.is_err());
    }

    #[tokio::test]
    async fn test_install_pkg() {
        let (env, _temp) = setup_test_env().await;
        
        // 创建测试包及其依赖
        create_test_package(&env, "test-pkg", "1.0.0").await;
        create_test_package(&env, "dep-pkg", "0.1.0").await;
        
        // 测试安装包(不包含依赖)
        let task_id = env.install_pkg("test-pkg#1.0.0", false).await.unwrap();
        assert_eq!(task_id, "test-pkg#1.0.0");
        
        // 等待任务完成
        env.wait_task(&task_id).await.unwrap();
        
        // 验证任务状态
        let tasks = env.install_tasks.lock().await;
        let task = tasks.get(&task_id).unwrap();
        assert!(matches!(task.status, InstallStatus::Completed));
        assert!(task.sub_tasks.is_empty());
    }

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
    async fn test_check_pkg_ready() {
        let (env, _temp) = setup_test_env().await;
        
        // 创建测试包
        create_test_package(&env, "test-pkg", "1.0.0").await;
        
        // 测试包就绪检查
        assert!(env.check_pkg_ready("test-pkg#1.0.0", false).await.is_ok());
        
        // 测试不存在的包
        assert!(env.check_pkg_ready("not-exist#1.0.0", false).await.is_err());
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
