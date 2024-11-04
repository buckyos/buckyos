// 0. 基于sqlite3作为数据库
// 1.批量产生未使用的激活码，激活码是32byte的随机字符串
// 2.提供注册接口，输入激活码，用户名，和一个用户提供的公钥。注册成功激活码会使用
//    用户名必须是全站唯一的，如果用户名被使用则返回注册失败。
// 3.提供用户设备信息的注册/更新/查询接口，设备信息包括设备的owner用户名,设备名，设备的did,设备的最新ip,以及字符串描述的设备信息，并保存有设备的创建时间和设备信息最后更新时间
#[allow(dead_code)]
use rusqlite::{params, Connection, OptionalExtension, Result};
use rand::Rng;
use std::{path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
use log::*;
use buckyos_kit::*;

pub fn get_sn_db_conn() -> Result<Connection> {
    let base_dir = PathBuf::from("/opt/web3_bridge/");
    let db_path = base_dir.join("sn_db.sqlite3");
    //make sure the dir exists
    let dir = db_path.parent().unwrap();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
    }

    let conn = Connection::open(db_path);
    if conn.is_err() {
        error!("Failed to open sn_db.sqlite3");
        return Err(conn.err().unwrap());
    }
    let conn = conn.unwrap();
    Ok(conn)
}

pub fn generate_activation_codes(conn: &Connection, count: usize) -> Result<Vec<String>> {
    let mut codes: Vec<String> = Vec::new();
    let mut stmt = conn.prepare("INSERT INTO activation_codes (code, used) VALUES (?1, 0)")?;
    for _ in 0..count {
        let code: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        codes.push(code.clone());
        stmt.execute(params![code])?;
    }

    Ok(codes)
}

pub fn check_active_code(conn: &Connection, active_code: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT used FROM activation_codes WHERE code = ?1")?;
    let used : Result<Option<i32>, rusqlite::Error> = stmt.query_row(params![active_code], |row| row.get(0));
    if used.is_err() {
        return Ok(false);
    }
    let used = used.unwrap();
    Ok(used.unwrap() == 0)
}

pub fn register_user(conn: &Connection, activation_code: &str, username: &str, public_key: &str, zone_config: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT used FROM activation_codes WHERE code = ?1")?;
    let used: Option<i32> = stmt.query_row(params![activation_code], |row| row.get(0))?;

    if let Some(0) = used {
        let mut stmt = conn.prepare("INSERT INTO users (username, public_key, activation_code, zone_config) VALUES (?1, ?2, ?3, ?4)")?;
        stmt.execute(params![username, public_key, activation_code, zone_config])?;
        
        let mut stmt = conn.prepare("UPDATE activation_codes SET used = 1 WHERE code = ?1")?;
        stmt.execute(params![activation_code])?;
        
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn is_user_exist(conn: &Connection, username: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM users WHERE username = ?1")?;
    let count: Option<i32> = stmt.query_row(params![username], |row| row.get(0))?;
    Ok(count.unwrap_or(0) > 0)
}

pub fn update_user_zone_config(conn: &Connection, username: &str, zone_config: &str) -> Result<()> {
    let mut stmt = conn.prepare("UPDATE users SET zone_config = ?1 WHERE username = ?2")?;
    stmt.execute(params![zone_config, username])?;
    Ok(())
}

pub fn get_user_info(conn: &Connection, username: &str) -> Result<Option<(String, String)>> {
    let mut stmt = conn.prepare("SELECT public_key, zone_config FROM users WHERE username = ?1")?;
    let user_info = stmt.query_row(params![username], |row| {
        Ok((row.get(0)?, row.get(1)?))
    }).optional()?;
    Ok(user_info)
}

pub fn register_device(conn: &Connection, username: &str, device_name: &str, did: &str, ip: &str, description: &str) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut stmt = conn.prepare("INSERT INTO devices (owner, device_name, did, ip, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)")?;
    stmt.execute(params![username, device_name, did, ip, description, now])?;
    Ok(())
}

pub fn update_device_by_did(conn: &Connection, did: &str, ip: &str, description: &str) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut stmt = conn.prepare("UPDATE devices SET ip = ?1, description = ?2, updated_at = ?3 WHERE did = ?4")?;
    stmt.execute(params![ip, description, now, did])?;
    Ok(())
}

pub fn update_device_by_name(conn: &Connection, owner: &str, device_name: &str, ip: &str, description: &str) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut stmt = conn.prepare("UPDATE devices SET ip = ?1, description = ?2, updated_at = ?3 WHERE device_name = ?4 AND owner = ?5")?;
    stmt.execute(params![ip, description, now, device_name, owner])?;
    Ok(())
}

pub fn query_device_by_name(conn: &Connection, owner: &str, device_name: &str) -> Result<Option<(String, String, String, String, String, u64, u64)>> {
    let mut stmt = conn.prepare("SELECT owner, device_name, did, ip, description, created_at, updated_at FROM devices WHERE device_name = ?1 AND owner = ?2")?;
    let device_info = stmt.query_row(params![device_name, owner], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?
        ))
    }).optional()?;
    Ok(device_info)
}

pub fn query_device(conn: &Connection, did: &str) -> Result<Option<(String, String, String, String, String, u64, u64)>> {
    let mut stmt = conn.prepare("SELECT owner, device_name, did, ip, description, created_at, updated_at FROM devices WHERE did = ?1")?;
    let device_info = stmt.query_row(params![did], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?
        ))
    }).optional()?;
    Ok(device_info)
}

pub fn initialize_database(conn: &Connection) -> Result<()> {
    // 创建激活码表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS activation_codes (
            code TEXT PRIMARY KEY,
            used INTEGER NOT NULL
        )",
        [],
    )?;

    // 创建用户表，并添加激活码字段
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            username TEXT PRIMARY KEY,
            public_key TEXT NOT NULL,
            activation_code TEXT,
            zone_config TEXT,
            FOREIGN KEY(activation_code) REFERENCES activation_codes(code)
        )",
        [],
    )?;

    // 为激活码字段创建索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_users_activation_code ON users (activation_code)",
        [],
    )?;

    // 创建设备表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS devices (
            owner TEXT NOT NULL,
            device_name TEXT NOT NULL,
            did TEXT PRIMARY KEY,
            ip TEXT NOT NULL,
            description TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        [],
    )?;

    // 为设备表的owner字段创建索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_devices_owner ON devices (owner)",
        [],
    )?;

    Ok(())
}




#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_main() -> Result<()> {
        let conn = get_sn_db_conn()?;
        initialize_database(&conn)?;
        // Example usage
        let codes = generate_activation_codes(&conn, 100)?;
        println!("codes: {:?}", codes);
        let first_code = codes.first().unwrap();
        let registration_success = register_user(&conn, first_code.as_str(), 
            "lzc", "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8", 
            "eyJhbGciOiJFZERTQSJ9.eyJkaWQiOiJkaWQ6ZW5zOmx6YyIsIm9vZHMiOlsib29kMSJdLCJzbiI6IndlYjMuYnVja3lvcy5pbyIsImV4cCI6MjA0NDgyMzMzNn0.Xqd-4FsDbqZt1YZOIfduzsJik5UZmuylknMiAxLToB2jBBzHHccn1KQptLhhyEL5_Y-89YihO9BX6wO7RoqABw")?;
        if registration_success {
            println!("User registered successfully.");
        } else {
            println!("Registration failed.");
        }
        let device_info_str =r#"{"hostname":"ood1","device_type":"ood","did":"did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc","ip":"192.168.1.86","sys_hostname":"LZC-USWORK","base_os_info":"Ubuntu 22.04 5.15.153.1-microsoft-standard-WSL2","cpu_info":"AMD Ryzen 7 5800X 8-Core Processor @ 3800 MHz","cpu_usage":0.0,"total_mem":67392299008,"mem_usage":5.7286677}"#;
        println!("device_info_str: {}",device_info_str);
        register_device(&conn, "lzc", "ood1", "did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc", "192.168.1.188", device_info_str)?;
        update_device_by_name(&conn, "lzc", "oo1", "75.4.200.194", device_info_str)?;
        
        if let Some(device_info) = query_device(&conn, "did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc")? {
            println!("Device info: {:?}", device_info);
        } else {
            println!("Device not found.");
        }
        
        Ok(())
    }
}
