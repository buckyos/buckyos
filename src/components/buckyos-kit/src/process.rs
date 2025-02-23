use log::*;
use package_lib::*;
use sysinfo::NetworkData;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::env;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::{exit, Stdio};
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Error, Debug)]
pub enum ServiceControlError {
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("No permission: {0}")]
    NoPermission(String),
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Pkg is not loaded")]
    PkgNotLoaded,
}

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

pub fn restart_program() -> std::io::Result<()> {
    let current_exe = env::current_exe()?;

    Command::new(current_exe)
        .args(env::args().skip(1))
        .spawn()?;

    exit(0);
}

pub async fn parse_script_file(path: &PathBuf) -> Result<(String, bool)> {
    let file = File::open(path)
        .await
        .map_err(|e| ServiceControlError::FileNotFound(e.to_string()))?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    let mut script_engine = String::new();
    let mut is_script = true;
    if let Ok(_) = reader.read_line(&mut first_line).await {
        if first_line.starts_with("#!") {
            script_engine = first_line.trim_start_matches("#!").trim().to_owned();
            // try to adjust script engine file path on windows
            if cfg!(target_os = "windows") {
                if let Some(name) = Path::new(&script_engine).file_name() {
                    script_engine = String::from(name.to_string_lossy());
                }
            }
        }
    }

    if script_engine.is_empty() {
        let extension = Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        match extension {
            "py" => script_engine = "python3".to_string(),
            "js" => script_engine = "node".to_string(),
            "sh" => script_engine = "sh".to_string(),
            _ => {
                script_engine = path.to_str().unwrap_or_default().to_string();
                is_script = false;
            }
        }
    }

    if script_engine.contains("python") {
        // 根据操作系统选择合适的python命令
        #[cfg(target_os = "windows")]
        let python_cmd = "python";
        #[cfg(target_os = "macos")]
        let python_cmd = "python3";
        #[cfg(target_os = "linux")]
        let python_cmd = "python3";
        
        script_engine = String::from(python_cmd);
    }

    Ok((script_engine, is_script))
}

pub async fn execute(
    path: &PathBuf,
    timeout_secs: u64,
    args: Option<&Vec<String>>,
    current_dir: Option<&PathBuf>,
    env_vars: Option<&HashMap<String, String>>,
) -> Result<(i32, Vec<u8>)> {
    let (script_engine, is_script) = parse_script_file(path).await?;
    let mut command = Command::new(script_engine);
    if is_script {
        command.arg(path);
    }

    if let Some(args) = args {
        command.args(args);
    }

    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    if let Some(env_vars) = env_vars {
        for (key, value) in env_vars {
            command.env(key, value);
        }
    }
    //println!("{:?}", command);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    info!("Executing: {:?}", command);

    let mut child = command
        .spawn()
        .map_err(|e| ServiceControlError::ReasonError(e.to_string()))?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");

    let output_future = async {
        let mut output = Vec::new();
        tokio::select! {
            result = read_stream(stdout) => {
                output.extend(result?);
            }
            result = read_stream(stderr) => {
                output.extend(result?);
            }
        }
        child.wait().await.map(|status| (status, output))
    };

    let result = timeout(Duration::from_secs(timeout_secs), output_future).await;

    match result {
        Ok(Ok((status, output))) => {
            let status_code = status.code().unwrap_or(-1);
            Ok((status_code, output))
        }
        Ok(Err(e)) => Err(ServiceControlError::ReasonError(e.to_string())),
        Err(_) => {
            // Timeout occurred, try to kill the process
            let _ = child.kill().await;
            Err(ServiceControlError::Timeout(
                "Script execution timed out".to_string(),
            ))
        }
    }
}

async fn read_stream<R: AsyncRead + Unpin>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await?;
    Ok(buffer)
}

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
            let new_media_info = self
                .pkg_env
                .load(&self.pkg_id);
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
        .await?;
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
