use std::path::Path;
use ini::Ini;
use shlex::Shlex;
use sysinfo::System;
use tokio::fs::read_to_string;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use crate::error::{into_smb_err, smb_err, SmbErrorCode, SmbResult};
use crate::samba::{SmbItem, SmbUserItem};

pub struct QAProcess {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    child: Child,
}

impl QAProcess {
    pub fn new(mut child: Child) -> Self {
        Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
        }
    }

    pub async fn answer(&mut self, question: &str, answer: &str) -> SmbResult<()> {
        // self.stdin.as_mut().unwrap().write_all(answer.as_bytes()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
        log::info!("{} start", question);
        let mut offset = 0;
        let mut buf = [0u8; 4096];
        let mut error_buf = [0u8; 4096];
        let mut error_offset = 0;
        loop {
            if offset == buf.len() || error_offset == error_buf.len() {
                return Err(smb_err!(SmbErrorCode::Failed, "Buffer overflow"));
            }

            tokio::select! {
                ret = self.stderr.as_mut().unwrap().read(&mut error_buf[error_offset..error_offset+1]) => {
                    match ret {
                        Ok(len) => {
                            if len == 0 {
                                return Err(smb_err!(SmbErrorCode::Failed, "EOF"));
                            }
                            error_offset += len;
                            let current = String::from_utf8_lossy(&error_buf[..error_offset]).to_string();
                            // log::info!("current err:{}", current);
                            if current.ends_with(question) {
                                let stdin = self.stdin.as_mut().ok_or(smb_err!(SmbErrorCode::Failed, "Failed to get stdin"))?;
                                // log::info!("write:{}", answer);
                                stdin.write_all(answer.as_bytes()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
                                stdin.write_all("\n".as_bytes()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
                                // log::info!("write:{} finish", answer);
                                break;
                            }
                        },
                        Err(e) => {
                            return Err(into_smb_err!(SmbErrorCode::Failed)(e))
                        }
                    }
                },
                ret = self.stdout.as_mut().unwrap().read(&mut buf[offset..offset+1]) => {
                    match ret {
                        Ok(len) => {
                            if len == 0 {
                                return Err(smb_err!(SmbErrorCode::Failed, "EOF"));
                            }
                            offset += len;
                            let current = String::from_utf8_lossy(&buf[..offset]).to_string();
                            // log::info!("current:{}", current);
                            if current.ends_with(question) {
                                let stdin = self.stdin.as_mut().ok_or(smb_err!(SmbErrorCode::Failed, "Failed to get stdin"))?;
                                // log::info!("write:{}", answer);
                                stdin.write_all(answer.as_bytes()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
                                stdin.write_all("\n".as_bytes()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
                                // log::info!("write:{} finish", answer);
                                break;
                            }
                        },
                        Err(e) => {
                            return Err(into_smb_err!(SmbErrorCode::Failed)(e))
                        }
                    }
                }
                _ = self.child.wait() => {
                    break;
                }
            }
        }
        log::info!("{} complete", question);
        Ok(())
    }

    pub async fn wait(&mut self) -> SmbResult<()> {
        let status = self.child.wait().await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
        if status.success() {
            Ok(())
        } else {
            let stderr = self.stderr.as_mut().ok_or(smb_err!(SmbErrorCode::Failed, "Failed to get stderr"))?;
            let mut error = Vec::new();
            stderr.read_to_end(&mut error).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;

            Err(smb_err!(SmbErrorCode::Failed, "{}", String::from_utf8_lossy(error.as_slice())))
        }
    }
}

async fn execute(cmd: &str) -> SmbResult<Vec<u8>> {
    log::info!("{}", cmd);
    let mut lexer = Shlex::new(cmd);
    let args: Vec<String> = lexer.by_ref().collect();
    let output = tokio::process::Command::new(args[0].as_str())
        .args(&args[1..])
        .output()
        .await
        .map_err(into_smb_err!(SmbErrorCode::Failed))?;
    if output.status.success() {
        log::info!("success:{}", String::from_utf8_lossy(output.stdout.as_slice()));
        Ok(output.stdout)
    } else {
        Err(smb_err!(SmbErrorCode::CmdReturnFailed, "{}", String::from_utf8_lossy(output.stderr.as_slice())))
    }
}

async fn spawn(cmd: &str) -> SmbResult<QAProcess> {
    log::info!("{}\n", cmd);
    let mut lexer = Shlex::new(cmd);
    let args: Vec<String> = lexer.by_ref().collect();
    let child = tokio::process::Command::new(args[0].as_str())
        .args(&args[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(into_smb_err!(SmbErrorCode::Failed))?;

    Ok(QAProcess::new(child))
}

pub async fn exist_system_user(user_name: &str) -> SmbResult<bool> {
    match execute(format!("id {}", user_name).as_str()).await {
        Ok(_) => {
            Ok(true)
        }
        Err(e) => {
            if e.code() == SmbErrorCode::CmdReturnFailed {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}
pub async fn add_system_user(user_name: &str) -> SmbResult<()> {
    execute(format!("useradd -s /sbin/nologin {}", user_name).as_str()).await?;
    Ok(())
}

pub async fn del_system_user(user_name: &str) -> SmbResult<()> {
    execute(format!("userdel {}", user_name).as_str()).await?;
    Ok(())
}

pub async fn add_smb_user(user_name: &str, password: &str) -> SmbResult<()> {
    let mut proc = spawn(format!("smbpasswd -a {}", user_name).as_str()).await?;
    proc.answer("New SMB password:", format!("{}", password).as_str()).await?;
    proc.answer("Retype new SMB password:", format!("{}", password).as_str()).await?;
    proc.wait().await?;
    Ok(())
}

pub async fn delete_smb_user(user_name: &str) -> SmbResult<()> {
    execute(format!("smbpasswd -x {}", user_name).as_str()).await?;
    Ok(())
}

pub async fn exist_smb_user(user_name: &str) -> SmbResult<bool> {
    let output = execute(format!("pdbedit -L | grep {}", user_name).as_str()).await?;
    let output = String::from_utf8_lossy(output.as_slice()).to_string();
    log::info!("output is:{}", output);
    Ok(true)
}

fn running_in_docker() -> bool {
    if let Ok(content) = std::fs::read_to_string("/proc/1/cgroup") {
        content.contains("/docker/")
    } else {
        false
    }
}

pub async fn restart_smb_service() -> SmbResult<()> {
    kill_smbd().await?;

    if running_in_docker() {
        execute("smbd").await?;
        return Ok(());
    }

    if let Some(version) = sysinfo::System::name() {
        if version.to_lowercase().contains("ubuntu") || version.to_lowercase().contains("debian") || version.to_lowercase().contains("centos") {
            execute("systemctl restart smb").await?;
        } else {
            execute("smbd").await?;
        }
    } else {
        execute("smbd").await?;
    }
    Ok(())
}

async fn kill_smbd() -> SmbResult<()> {
    let smbd_pid_file =  Path::new("/run/samba/smbd.pid");
    if smbd_pid_file.exists() {
        let pid = read_to_string(smbd_pid_file).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
        let pid = pid.trim();
        execute(format!("kill -9 {}", pid).as_str()).await?;
    } else {
        // 通过进程列表关闭
        let mut system = System::new_all();
        system.refresh_all();
        for (_, process) in system.processes() {
            if process.name() == "smbd" {
                execute(format!("kill -9 {}", process.pid()).as_str()).await?;
            }
        }
    }
    Ok(())
}

pub async fn stop_smb_service() -> SmbResult<()> {
    if let Some(version) = sysinfo::System::name() {
        if version.to_lowercase().contains("ubuntu") || version.to_lowercase().contains("debian") || version.to_lowercase().contains("centos") {
            execute("systemctl stop smbd").await?;
        } else {
            kill_smbd().await?;
        }
    } else {
        kill_smbd().await?;
    }
    Ok(())
}

pub async fn load_sub_smb_conf() -> SmbResult<Vec<Ini>> {
    let config_path = Path::new("/etc/samba/smb.conf.d");
    if !config_path.exists() || !config_path.is_dir() {
        return Ok(Vec::new());
    }

    let mut entrys = tokio::fs::read_dir(config_path).await
        .map_err(into_smb_err!(SmbErrorCode::LoadSmbConfFailed, "{}", config_path.to_string_lossy().to_string()))?;

    let mut list = Vec::new();
    while let Some(entry) = entrys.next_entry().await
        .map_err(into_smb_err!(SmbErrorCode::LoadSmbConfFailed, "{}", config_path.to_string_lossy().to_string()))? {
        if let Some(ext) = entry.path().extension() {
            if ext == "conf" {
                let config = Ini::load_from_file(entry.path())
                    .map_err(into_smb_err!(SmbErrorCode::LoadSmbConfFailed, "{}", entry.path().to_string_lossy().to_string()))?;
                list.push(config);
            }
        }
    }
    Ok(list)
}

async fn generate_smb_conf(smb_list: &Vec<SmbItem>) -> SmbResult<()> {
    let mut conf = Ini::new();
    conf.with_section(Some("global"))
        .set("workgroup", "WORKGROUP")
        .set("server string", "Samba Server")
        .set("security", "user")
        .set("server role", "standalone server")
        .set("pam password change", "yes")
        .set("map to guest", "bad user")
        .set("usershare allow guests", "yes")
        .set("log level", "1000")
        .set("log file", "/var/log/samba/samba.log");

    let ini_list = load_sub_smb_conf().await?;
    for ini in ini_list.iter() {
        for (section, prop) in ini.iter() {
            if let Some(section) = section {
                let mut sec = conf.with_section(Some(section));
                for (key, value) in prop.iter() {
                    sec.set(key, value);
                }
            }
        }
    }

    for item in smb_list.iter() {
        let mut sec = conf.with_section(Some(item.smb_name.as_str()));
        sec.set("path", &item.path);
        sec.set("valid users", item.allow_users.join(" "));
        sec.set("writable", "yes");
        sec.set("browsable", "yes");
        sec.set("guest ok", "no");
    }

    conf.write_to_file("/etc/samba/smb.conf").map_err(into_smb_err!(SmbErrorCode::LoadSmbConfFailed, "{}", "/etc/samba/smb.conf"))?;
    Ok(())
}

pub async fn update_samba_conf(_remove_users: Vec<SmbUserItem>, new_all_users: Vec<SmbUserItem>, _remove_list: Vec<SmbItem>, new_samba_list: Vec<SmbItem>) -> SmbResult<()> {
    for item in new_all_users.iter() {
        let _ = delete_smb_user(item.user.as_str()).await;
        if !exist_system_user(item.user.as_str()).await? {
            add_system_user(item.user.as_str()).await?;
        }
        add_smb_user(item.user.as_str(), item.password.as_str()).await?;
    }

    generate_smb_conf(&new_samba_list).await?;
    restart_smb_service().await?;
    Ok(())
}

pub async fn check_samba_status() -> i32 {
    if !Path::new("/usr/sbin/smbd").exists() {
        return 255;
    }
    let mut system = System::new_all();
    system.refresh_all();

    let mut is_smbd_running = false;
    for _ in system.processes_by_name("smbd".as_ref()) {
        is_smbd_running = true;
    }

    if is_smbd_running {
        0
    } else {
        1
    }
}
