use log::*;
use package_lib::*;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::collections::HashMap;
use thiserror::Error;
use buckyos_kit::*;
use buckyos_api::ServiceInstanceState;


type Result<T> = std::result::Result<T, ServiceControlError>;

pub struct ServicePkg {
    pub pkg_id: String,
    pub pkg_env_path: PathBuf,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Mutex<Option<MediaInfo>>,
}


impl Default for ServicePkg {
    fn default() -> Self {
        Self {
            pkg_id: "".to_string(),
            pkg_env_path: PathBuf::from(""),
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
            pkg_env_path: env_path,
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
        }
    }

    pub async fn try_load(&self) -> bool {
        let mut media_info = self.media_info.lock().await;
        if media_info.is_none() {
            let pkg_env = PackageEnv::new(self.pkg_env_path.clone()); 
            let new_media_info = pkg_env.load(&self.pkg_id).await;
            if new_media_info.is_ok() {
                debug!("load service pkg {} success", self.pkg_id);
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

    pub async fn execute_operation(&self, op_name: &str, params: Option<&Vec<String>>) -> Result<i32> {
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
            1200,
            params,
            self.current_dir.as_ref(),
            Some(&self.env_vars),
        )
            .await
            .map_err(|e| {
                error!("# execute {} failed! {}", op_file.display(), e);
                return ServiceControlError::ReasonError(e.to_string());
            })?;

        let params_str = params.map(|p| p.join(" ")).unwrap_or_default();
        if result == 0 {
            info!(
                "# run {} {} => {} \n\t {}",
                op_file.display(),
                params_str,
                result,
                String::from_utf8_lossy(&output)
            );
        } else {
            info!(
                "# run {} {} => {} \n\t {}",
                op_file.display(),
                params_str,
                result,
                String::from_utf8_lossy(&output)
            ); 
        }
        Ok(result)
    }

    pub async fn start(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load().await;
        let result = self.execute_operation( "start", params).await?;
        Ok(result)
    }

    pub async fn stop(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load().await;
        let result = self.execute_operation("stop", params).await?;
        Ok(result)
    }

    pub async fn status(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState> {
        let pkg_env = PackageEnv::new(self.pkg_env_path.clone()); 
        let media_info = pkg_env.load(&self.pkg_id).await;
        if media_info.is_err() {
            info!("pkg {} not exist", self.pkg_id);
            return Ok(ServiceInstanceState::NotExist);
        }
        let media_info = media_info.unwrap();
        let mut media_info_lock = self.media_info.lock().await;
        *media_info_lock = Some(media_info);
        drop(media_info_lock);
        let result = self.execute_operation("status", params).await?;
        match result {
            0 => Ok(ServiceInstanceState::Started),
            255 => Ok(ServiceInstanceState::NotExist),
            254 => Ok(ServiceInstanceState::Deploying),
            _ => Ok(ServiceInstanceState::Stopped),
        }
    }
}
