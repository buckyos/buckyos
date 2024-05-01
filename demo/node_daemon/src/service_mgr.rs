use crate::pkg_mgr::*;
use crate::run_item::*;
use async_trait::async_trait;
use log::*;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
pub struct ServiceItem {
    name: String,
    version: String,
    pkg_id: String,
}

#[async_trait]
impl RunItemControl for ServiceItem {
    fn get_item_name(&self) -> Result<String> {
        return Ok(self.name.clone());
    }

    async fn deploy(&self, params: Option<&RunItemParams>) -> Result<()> {
        // 部署文件系统时需要机器名，以root权限运行脚本，默认程序本身应该是root权限
        let env = PackageEnv::new(PathBuf::from("/buckyos/service"));
        let media_info = env.load_pkg(&self.name).await.map_err(|err| {
            ControlRuntItemErrors::ExecuteError(
                format!("deploy service {} error", self.name),
                err.to_string(),
            )
        })?; // 从pkg_id获取media_info
        let deploy_sh_file = media_info.full_path.join("/deploy.sh");
        //run_cmd(deploy_sh_file)
        Self::run_shell_script_with_args(
            deploy_sh_file.to_str().unwrap(),
            &[
                params.unwrap().node_id.clone(),
                params.unwrap().node_ip.clone(),
            ],
        )
    }

    async fn remove(&self, params: Option<&RunItemParams>) -> Result<()> {
        Ok(())
    }

    async fn update(&self, params: Option<&RunItemParams>) -> Result<String> {
        Ok(String::from("1.0.1"))
    }

    async fn start(&self, params: Option<&RunItemParams>) -> Result<()> {
        let scrpit_path = self.get_script_path("start.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
        Ok(())
    }

    async fn stop(&self, params: Option<&RunItemParams>) -> Result<()> {
        let scrpit_path = self.get_script_path("stop.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
        unimplemented!();
    }

    async fn get_state(&self, params: Option<&RunItemParams>) -> Result<RunItemState> {
        //pkg_media_info= env.load_pkg(&self.pkg_id)
        //if pkg_media_info.is_none(){
        //    return RunItemState::NotExist
        //}

        let scrpit_path = self.get_script_path("get_state.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
        //根据返回值判断状态
        unimplemented!()
    }
}

pub async fn create_service_item_from_config(service_cfg: &str) -> Result<ServiceItem> {
    //parse servce_cfg to json
    //create ServiceItem from josn
    //return ServiceItem
    unimplemented!();
}

impl ServiceItem {
    pub fn new(name: String, version: String, pkg_id: String) -> Self {
        ServiceItem {
            name,
            version,
            pkg_id,
        }
    }

    pub fn get_script_path(&self, script_name: &str) -> Result<String> {
        //media_info = env.load_pkg(&self.name)
        //script_path = media_info.folder + "/" + script_name
        //return script_path
        unimplemented!();
    }

    fn run_shell_script_with_args(script_path: &str, args: &[String]) -> Result<()> {
        let mut command = Command::new("sh");
        command.arg(script_path);
        for arg in args {
            command.arg(arg);
        }

        // 设置标准输出和标准错误为管道
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|err| {
            error!("launch script {} error: {}", script_path, err);
            ControlRuntItemErrors::ExecuteError(
                format!("launch script {} error", script_path),
                err.to_string(),
            )
        })?;

        // 获取标准输出的管道
        if let Some(stdout) = child.stdout.take() {
            let stdout_reader = BufReader::new(stdout);
            for line in stdout_reader.lines() {
                match line {
                    Ok(line) => println!("{}", line),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
        }

        // 获取标准错误的管道
        if let Some(stderr) = child.stderr.take() {
            let stderr_reader = BufReader::new(stderr);
            for line in stderr_reader.lines() {
                match line {
                    Ok(line) => eprintln!("{}", line),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
        }

        // 等待子进程结束
        let status = child.wait().map_err(|err| {
            error!("wait script complete {} error: {}", script_path, err);
            ControlRuntItemErrors::ExecuteError(
                format!("wait script complete {} error", script_path),
                err.to_string(),
            )
        })?;

        if status.success() {
            info!("exec script {} success", script_path);
        } else {
            error!("exec script {} failed. status: {}", script_path, status);
        }

        Ok(())
    }
}
