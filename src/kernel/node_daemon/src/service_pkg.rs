use log::*;
use package_lib::*;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::collections::HashMap;
use thiserror::Error;
use buckyos_kit::*;



#[derive(PartialEq)]
pub enum ServiceState {
    //InstllDeps,
    Deploying,
    //DeployFailed(String,u32), //error message,failed count
    NotExist,
    Started,
    Stopped,
}

type Result<T> = std::result::Result<T, ServiceControlError>;

pub struct ServicePkg {
    pub pkg_id: String,
    pub pkg_env: PackageEnv,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Mutex<Option<MediaInfo>>,
}


impl Default for ServicePkg {
    fn default() -> Self {
        Self {
            pkg_id: "".to_string(),
            pkg_env: PackageEnv::new(PathBuf::from("")),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
        }
    }
}
impl ServicePkg {
    pub fn new(pkg_id: String, env_path: PathBuf) -> Self {
        Self {
            pkg_id,
            pkg_env: PackageEnv::new(env_path),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
        }
    }

    pub async fn try_load(&self,index_db_only: bool) -> bool {
        let mut media_info = self.media_info.lock().await;
        if media_info.is_none() {
            //todo: use index_db_only to load media_info
            //todo: 是否需要在接口上进行区分？ 还是只需要兼容无index_db的开发模式就好？即在生产环境一定会有index_db
            let new_media_info = self
                .pkg_env
                .load(&self.pkg_id).await;
            
            if new_media_info.is_ok() {
                info!("load pkg {} success", self.pkg_id);
                let new_media_info = new_media_info.unwrap();
                *media_info = Some(new_media_info);
                return true;
            }
        }
        false
    }

    pub fn set_context(
        &mut self,
        current_dir: Option<&PathBuf>,
        env_vars: Option<&HashMap<String, String>>,
    ) {
        if let Some(current_dir) = current_dir {
            self.current_dir = Some(current_dir.clone());
        }
        if let Some(env_vars) = env_vars {
            self.env_vars = env_vars.clone();
        }
    }

    async fn execute_operation(&self, op_name: &str, params: Option<&Vec<String>>) -> Result<i32> {
        //let media_info = self.media_info.clone().unwrap();
        let media_info = self.media_info.lock().await;
        let media_info = media_info.as_ref();
        if media_info.is_none() {
            return Err(ServiceControlError::PkgNotLoaded);
        }
        let media_info = media_info.unwrap();

        let op_file = media_info.full_path.join(op_name);
        let (result, output) = execute(
            &op_file,
            5,
            params,
            self.current_dir.as_ref(),
            Some(&self.env_vars),
        )
        .await
        .map_err(|e| {
            error!("execute {} failed! {}", op_file.display(), e);
            return ServiceControlError::ReasonError(e.to_string());
        })?;
        info!(
            "execute {} ==> result: {} \n\t {}",
            op_file.display(),
            result,
            String::from_utf8_lossy(&output)
        );
        Ok(result)
    }

    pub async fn start(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load(false).await;
        let result = self.execute_operation( "start", params).await?;
        Ok(result)
    }

    pub async fn stop(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load(false).await;
        let result = self.execute_operation("stop", params).await?;
        Ok(result)
    }

    pub async fn status(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        self.try_load(true).await;
        if self.media_info.lock().await.is_none() {
            info!("pkg {} not exist", self.pkg_id);
            return Ok(ServiceState::NotExist);
        }
        let result = self.execute_operation("status", params).await?;
        match result {
            0 => Ok(ServiceState::Started),
            -1 => Ok(ServiceState::NotExist),
            -2 => Ok(ServiceState::Deploying),
            _ => Ok(ServiceState::Stopped),
        }
    }
}
