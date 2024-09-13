use tokio::time::{timeout, Duration};
use tokio::process::{Command};
use log::*;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{BufReader, AsyncBufReadExt, AsyncReadExt, AsyncRead};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use package_manager::*;

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

pub async fn execute(path: &PathBuf, timeout_secs: u64, args: Option<&Vec<String>>,
                    current_dir: Option<&PathBuf>, env_vars: Option<&HashMap<String, String>>) -> Result<(i32, Vec<u8>)> {
    let file = File::open(path).await.map_err(|e| ServiceControlError::FileNotFound(e.to_string()))?;
    let mut reader = BufReader::new(file);
    let command_str: String;
    let mut command = Command::new(path.to_str().unwrap_or_default());

    let mut first_line = String::new();
    let mut read_first_line = false;
    let read_result = reader.read_line(&mut first_line).await;
    if read_result.is_ok() {
        if first_line.starts_with("#!") {
            let script_engine = first_line.trim_start_matches("#!").trim();
            //得到脚本引擎的可执行文件名
            let script_engine_path = Path::new(script_engine);
            //执行脚本引擎
            command = Command::new(script_engine_path.file_name().unwrap().to_str().unwrap());
            command.arg(path);
            read_first_line = true;
            //info!("start run script execute {} {}...", script_engine_path.file_name().unwrap().to_str().unwrap(),path.to_string_lossy());
        }
    }

    if !read_first_line {
        let extension = Path::new(path).extension().and_then(|s| s.to_str()).unwrap_or("");
        let mut is_known_script = true;
        match extension {
            "py" => command_str = "python3".to_string(),
            "js" => command_str = "node".to_string(),
            "sh" => {
                command_str = "sh".to_string();
            },
            _ => {
                command_str = path.to_str().unwrap_or_default().to_string();
                is_known_script = false;
            }
        }

        command = Command::new(command_str);
        if is_known_script {
            command.arg(path);
        }     
    }

    if let Some(args) = args {
        for arg in args {
            command.arg(arg);
        }
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
    
    let mut child = command.spawn().map_err(|e| ServiceControlError::ReasonError(e.to_string()))?;

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
            Err(ServiceControlError::Timeout("Script execution timed out".to_string()))
        }
    }
}

async fn read_stream<R: AsyncRead + Unpin>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await?;
    Ok(buffer)
}

pub struct ServicePkg {
    pub pkg_id : String,
    pub pkg_env: PackageEnv,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Option<MediaInfo>,
}

impl Default for ServicePkg {
    fn default() -> Self {
        Self {
            pkg_id: "".to_string(),
            pkg_env: PackageEnv::new(PathBuf::from("")),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: None,
        }
    }
}
impl ServicePkg {
    pub fn new(pkg_id: String,env_path: PathBuf) -> Self {
        Self {
            pkg_id,
            pkg_env: PackageEnv::new(env_path),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: None,
        }
    }

    pub async fn load(&mut self) -> Result<MediaInfo> {
        let media_info = self.pkg_env.load(&self.pkg_id)
            .await.map_err(|e| ServiceControlError::ReasonError(e.to_string()))?;
        self.media_info = Some(media_info.clone());
        Ok(media_info)
    }

    pub fn set_context(&mut self,current_dir:Option<&PathBuf>,env_vars:Option<&HashMap<String, String>>) {
        if let Some(current_dir) = current_dir {
            self.current_dir = Some(current_dir.clone());
        }
        if let Some(env_vars) = env_vars {
            self.env_vars = env_vars.clone();
        }
    }

    async fn execute_operation(&self, op_name: &str,parms:Option<&Vec<String>>) -> Result<i32> {
        if self.media_info.is_none() {
            return Err(ServiceControlError::ReasonError("media info is not loaded".to_string()));
        }
        let media_info = self.media_info.clone().unwrap();
        let op_file = media_info.full_path.join(op_name);
        //info!("start execute {} ...", op_file.display());
        let (result, output) = execute(&op_file, 5, parms,
            self.current_dir.as_ref(), Some(&self.env_vars)).await?;
        info!("execute {} ==> result: {} \n\t {}", op_file.display(), result, String::from_utf8_lossy(&output));
        Ok(result)
    }

    pub async fn start(&self,parms:Option<&Vec<String>>) -> Result<i32> {
        let result = self.execute_operation("start",parms).await?;
        Ok(result)
    }

    pub async fn stop(&self,parms:Option<&Vec<String>>) -> Result<i32> {
        let result = self.execute_operation("stop",parms).await?;
        Ok(result)
    }

    pub async fn status(&self,parms:Option<&Vec<String>>) -> Result<ServiceState> {
        let result = self.execute_operation("status",parms).await?;
        match result {
            0 => Ok(ServiceState::Started),
            -1 => Ok(ServiceState::NotExist),
            -2 => Ok(ServiceState::Deploying),
            _ => Ok(ServiceState::Stopped)
        }
    }
}