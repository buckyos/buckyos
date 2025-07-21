use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use sfo_io::error::SfoIOErrorCode;
use sfo_io::execute;
use crate::error::{into_smb_err, SmbErrorCode, SmbResult};
use crate::samba::{SmbItem, SmbUserItem};

pub async fn update_samba_conf(remove_users: Vec<SmbUserItem>, new_all_users: Vec<SmbUserItem>, remove_list: Vec<SmbItem>, new_samba_list: Vec<SmbItem>) -> SmbResult<()> {
    for item in remove_users.iter() {
        if is_buckyos_user(item.user.as_str()).await? {
            execute(format!(r#"pwpolicy -u {} -sethashtypes SMB-NT off"#, item.user).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
            execute(format!(r#"sudo dscl . -passwd /Users/"{}" {}"#, item.user, item.password).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
        }
    }

    for item in new_all_users.iter() {
        if exist_system_user(item.user.as_str()).await? {
            if is_buckyos_user(item.user.as_str()).await? {
                set_system_user_passwd(item.user.as_str(), item.password.as_str()).await?;
            }
        } else {
            add_system_user(item.user.as_str(), item.password.as_str(), 20).await?;
        }
    }

    for item in remove_list.iter() {
        remove_share(get_share_record_name(item.smb_name.as_str()).as_str(), item.path.as_str()).await?
    }

    for item in new_samba_list.iter() {
        if !is_sharing_path_or_record_name(item.path.as_str(), get_share_record_name(item.smb_name.as_str()).as_str()).await? {
            add_share(item.smb_name.as_str(), get_share_record_name(item.smb_name.as_str()).as_str(), item.path.as_str(), item.allow_users.clone()).await?;
        }
    }
    Ok(())
}

fn get_share_record_name(name: &str) -> String {
    format!("buckyos_{}", name)
}

pub async fn stop_smb_service() -> SmbResult<()> {
    let sharing_list = get_sharing_list().await?;
    for (name, item) in sharing_list.iter() {
        if name.starts_with("buckyos_") {
            remove_share(name.as_str(), item.path.as_str()).await?
        }
    }
    Ok(())
}

pub async fn check_samba_status() -> i32 {
    match is_share_opened().await {
        Ok(opened) => {
            if opened {
                0
            } else {
                1
            }
        }
        Err(_) => {
            1
        }
    }
}

async fn exist_system_user(user_name: &str) -> SmbResult<bool> {
    match execute(format!("id {}", user_name).as_str()).await {
        Ok(_) => {
            Ok(true)
        }
        Err(e) => {
            if e.code() == SfoIOErrorCode::CmdReturnFailed {
                Ok(false)
            } else {
                Err(into_smb_err!(SmbErrorCode::Failed)(e))
            }
        }
    }
}

//pwpolicy -u SomeUser -sethashtypes SMB-NT off
// sudo launchctl enable system/com.apple.smbd
// ls -lde /Users/renwu/data
// sudo dscl . -read /SharePoints
// sudo dscl . -list /SharePoints
async fn add_system_user(user_name: &str, passwd: &str, group_id: u32) -> SmbResult<()> {
    execute(format!(r#"sudo dscl . -create /Users/"{}" IsHidden 1"#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -create /Users/"{}" UserShell /bin/false"#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -create /Users/"{}" RealName "{} buckyos share""#, user_name, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let unique_id = get_next_system_id().await?;
    execute(format!(r#"sudo dscl . -create /Users/"{}" UniqueID "{}""#, user_name, unique_id).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -create /Users/"{}" PrimaryGroupID {}"#, user_name, group_id).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -passwd /Users/"{}" {}"#, user_name, passwd).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"pwpolicy -u {} -sethashtypes SMB-NT on"#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -passwd /Users/"{}" {}"#, user_name, passwd).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;

    Ok(())
}

async fn set_system_user_passwd(user_name: &str, password: &str) -> SmbResult<()> {
    execute(format!(r#"pwpolicy -u {} -sethashtypes SMB-NT on"#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -passwd /Users/"{}" {}"#, user_name, password).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}
async fn remove_system_user(user_name: &str) -> SmbResult<()> {
    execute(format!(r#"sudo dscl . -delete /Users/"{}""#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

pub async fn add_smb_user(user_name: &str, password: &str) -> SmbResult<()> {
    execute(format!(r#"sudo smbpasswd -a "{}" "{}""#, user_name, password).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

async fn get_next_system_id() -> SmbResult<u32> {
    let output = execute("dscl . -list /users UniqueID").await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    let max_uid = output_str
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .filter_map(|s| s.parse::<u32>().ok())
        .max()
        .unwrap_or(500); // 默认从501开始

    Ok(if max_uid < 501 { 501 } else { max_uid + 1 })
}

async fn get_user_realname(user_name: &str) -> SmbResult<String> {
    let output = execute(format!(r#"dscl . -read /Users/"{}" RealName"#, user_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    let mut lines = output_str.lines();
    let first_line = lines.next();
    let second_line = lines.next();
    if second_line.is_none() {
        Ok(first_line.unwrap_or("").replace(r#"RealName: "#, "").trim().to_string())
    } else {
        Ok(second_line.unwrap_or("").trim().to_string())
    }
}

async fn get_next_group_id() -> SmbResult<u32> {
    let output = execute("dscl . -list /Groups PrimaryGroupID").await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    let max_uid = output_str
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .filter_map(|s| s.parse::<u32>().ok())
        .max()
        .unwrap_or(800);

    Ok(if max_uid < 501 { 501 } else { max_uid + 1 })
}

pub struct GroupInfo {
    group_id: u32,
}
async fn get_group_info(group_name: &str) -> SmbResult<Option<GroupInfo>> {
    let output = match execute(format!(r#"dscl . -read /Groups/"{}""#, group_name).as_str()).await {
        Ok(output) => output,
        Err(e) => {
            return if e.msg().contains("eDSRecordNotFound") {
                Ok(None)
            } else {
                Err(into_smb_err!(SmbErrorCode::Failed)(e))
            }
        }
    };
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    let lines = output_str.lines();
    let mut group_id = None;
    for line in lines {
        let elments = line.split(":").collect::<Vec<&str>>();
        if elments.len() == 2 {
            if elments[0].trim() == "PrimaryGroupID" {
                group_id = Some(elments[1].trim().parse::<u32>().map_err(into_smb_err!(SmbErrorCode::Failed, "parse group id {} failed", elments[1].trim()))?);
                break;
            }
        }
    }

    if group_id.is_none() {
        return Ok(None)
    }
    Ok(Some(GroupInfo {
        group_id: group_id.unwrap(),
    }))
}

async fn create_group(group_name: &str) -> SmbResult<()> {
    let group_id = get_next_group_id().await?;
    execute(format!(r#"sudo dscl . -create /Groups/"{}""#, group_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    execute(format!(r#"sudo dscl . -create /Groups/"{}" PrimaryGroupID "{}""#, group_name, group_id).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

async fn is_buckyos_user(user_name: &str) -> SmbResult<bool> {
    let realname = get_user_realname(user_name).await?;
    Ok(realname.contains("buckyos share"))
}

async fn add_share(share_name: &str, record_name: &str, path: &str, allow_users: Vec<String>) -> SmbResult<()> {
    execute(format!(r#"sudo sharing -a "{}" -S {} -n "{}" -s 001 -g 000"#, path, share_name, record_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    for user in allow_users {
        execute(format!(r#"sudo chmod -R +a '{} allow list,add_file,search,add_subdirectory,delete_child,readattr,writeattr,readextattr,writeextattr,readsecurity,file_inherit,directory_inherit' {}"#, user, path).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    }
    Ok(())
}

async fn remove_share(record_name: &str, path: &str) -> SmbResult<()> {
    execute(format!(r#"sudo sharing -r "{}""#, record_name).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let acl_items = get_acl(path).await?;
    for acl_item in acl_items {
        remove_acl(path, &acl_item).await?;
    }
    Ok(())
}

async fn remove_acl(path: &str, acl_item: &ACLItem) -> SmbResult<()> {
    execute(format!(r#"sudo chmod -R -a '{} {}' {}"#, acl_item.user, acl_item.permissions, path).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

struct ACLItem {
    user: String,
    permissions: String,
}

async fn get_acl(path: &str) -> SmbResult<Vec<ACLItem>> {
    let output = execute(format!("ls -lde {}", path).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    let mut lines = output_str.lines();
    let mut acl_items = Vec::new();
    for line in lines {
        let elments = line.split_whitespace().collect::<Vec<&str>>();
        let mut item = ACLItem {
            user: "".to_string(),
            permissions: "".to_string(),
        };
        if elments.len() == 4 {
            if elments[1].contains("user:") {
                item.user = elments[1].replace("user:", "").trim().to_string();
                item.permissions = format!("{} {}", elments[2], elments[3]);
            }
            acl_items.push(item);
        }
    }
    Ok(acl_items)
}

async fn is_share_opened() -> SmbResult<bool> {
    let output = execute("sudo launchctl list").await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let output_str = String::from_utf8_lossy(output.as_slice()).to_string();
    Ok(output_str.contains("com.apple.smbd"))
}

async fn open_share() -> SmbResult<()> {
    execute("sudo launchctl load -w /System/Library/LaunchDaemons/com.apple.smbd.plist").await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct SharingItem {
    path: String,
    smb_guest_access: u32,
    smb_name: String,
    smb_read_only: u32,
    smb_sealed: u32,
    smb_shared: u32,
}
async fn get_sharing_list() -> SmbResult<HashMap<String, SharingItem>> {
    let output = execute("sharing -l -f json").await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    let items = serde_json::from_slice(output.as_slice()).map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(items)
}

async fn is_sharing_path(path: &str) -> SmbResult<bool> {
    let sharing_list = get_sharing_list().await?;
    for (_, item) in sharing_list {
        if item.path == path {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn is_sharing_record_name(name: &str) -> SmbResult<bool> {
    let sharing_list = get_sharing_list().await?;
    Ok(sharing_list.contains_key(name))
}

async fn is_sharing_path_or_record_name(path: &str, name: &str) -> SmbResult<bool> {
    let sharing_list = get_sharing_list().await?;
    if sharing_list.contains_key(name) {
        return Ok(true);
    }
    for (_, item) in sharing_list {
        if item.path == path {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn get_sharing_record_name(path: &str) -> SmbResult<Option<String>> {
    let sharing_list = get_sharing_list().await?;
    for (name, item) in sharing_list {
        if item.path == path {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

async fn get_sharing_path(share_record: &str) -> SmbResult<Option<String>> {
    let sharing_list = get_sharing_list().await?;
    Ok(sharing_list.get(share_record).map(|item| item.path.clone()))
}

async fn set_path_owner(path: &str, user_name: &str) -> SmbResult<()> {
    execute(format!(r#"sudo chown -R "{}" "{}""#, user_name, path).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}

async fn set_path_permission(path: &str, permission: &str) -> SmbResult<()> {
    execute(format!(r#"sudo chmod -R "{}" "{}""#, permission, path).as_str()).await.map_err(into_smb_err!(SmbErrorCode::Failed))?;
    Ok(())
}
