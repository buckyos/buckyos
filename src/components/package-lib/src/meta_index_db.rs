/*
表一 pkg_metas，注意pkg-meta里可以保存json字符串,也可保存jwt字符串，author-pk可以为空
metaobjid,pkg-meta,author,author-pk,update_time, 

表二 pkg_versions
pkgname, version, metaobjid, tag, update_time
pkgname-version 形成了唯一的key

表三 author_info,为了使用方便，已经把author_pk从author_owner_config中分离了
author_name,author_pk,author_owner_config,author_zone_config


查询接口
//pkg_name可以是 author/pkg_name 的形式
get_pkg_meta(pkg_name,author,version),version不填表示最新版本
get_pkg_meta_by_tag(pkg_name,tag)
get_author_info(author_name)
get_all_pkg_versions(pkg_name)


修改接口
add_pkg_meta(metaobjid,pkg-meta,author,author-pk) 
set_pkg_version(pkgname,version,metaobjid)
set_author_info(author_name,author_owner_config,author_zone_config)
*/

use log::*;
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use chrono::Utc;
use semver::*;
use crate::error::*;
use crate::meta::*;
use crate::package_id::*;

#[derive(Debug)]
pub struct MetaIndexDb {
    pub db_path: PathBuf,
}

impl MetaIndexDb {
    pub fn new(db_path: PathBuf) -> PkgResult<Self> {
        // 初始化时可以检查数据库文件是否可访问，并创建必要的表和索引
        let conn = Self::create_connection(&db_path)?;
        
        // 创建 pkg_metas 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pkg_metas (
                metaobjid TEXT PRIMARY KEY,
                pkg_meta TEXT NOT NULL,
                author TEXT NOT NULL,
                author_pk TEXT NOT NULL,
                update_time INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        // 创建 pkg_versions 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pkg_versions (
                pkgname TEXT NOT NULL,
                version TEXT NOT NULL,
                version_int INTEGER NOT NULL,
                metaobjid TEXT NOT NULL,
                tag TEXT,
                update_time INTEGER NOT NULL,
                PRIMARY KEY (pkgname, version)
            )",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        // 创建 author_info 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS author_info (
                author_name TEXT PRIMARY KEY,
                author_owner_config TEXT,
                author_zone_config TEXT
            )",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        // 创建索引
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_versions_pkgname ON pkg_versions (pkgname)",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_metas_author ON pkg_metas (author)",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_versions_version_int ON pkg_versions (pkgname, version_int)",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        // 完成初始化后关闭连接
        drop(conn);
        
        Ok(MetaIndexDb { db_path })
    }
    
    // 创建数据库连接的辅助方法
    fn create_connection(db_path: &PathBuf) -> PkgResult<Connection> {
        Connection::open_with_flags(db_path, 
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE 
            | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX
        ).map_err(|e| PkgError::SqlError(e.to_string()))
    }

    // 查询接口

    /// 获取包元数据
    /// 如果version为None，则返回最新版本
    pub fn get_pkg_meta(&self, pkg_id: &str, author: Option<&str>, version: Option<&str>) -> PkgResult<Option<(String, PackageMeta)>> {
        unimplemented!()
    }

    /// 获取作者信息
    pub fn get_author_info(&self, author_name: &str) -> PkgResult<Option<(String, Option<String>, Option<String>)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        conn.query_row(
            "SELECT author_name, author_owner_config, author_zone_config FROM author_info WHERE author_name = ?",
            params![author_name],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        ).optional().map_err(|e| PkgError::SqlError(e.to_string()))
    }

    /// 获取包的所有版本
    pub fn get_all_pkg_versions(&self, pkg_name: &str) -> PkgResult<Vec<(String, String, Option<String>)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let mut stmt = conn.prepare(
            "SELECT version, metaobjid, tag FROM pkg_versions WHERE pkgname = ? ORDER BY update_time DESC"
        ).map_err(|e| PkgError::SqlError(e.to_string()))?;

        let versions = stmt.query_map(params![pkg_name], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;

        let mut result = Vec::new();
        for version in versions {
            result.push(version.map_err(|e| PkgError::SqlError(e.to_string()))?);
        }

        Ok(result)
    }

    /// 获取版本范围
    pub fn get_versions_in_range(&self, pkg_name: &str, min_version: Option<&str>, max_version: Option<&str>) -> PkgResult<Vec<(String, String, Option<String>)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let mut conditions = vec!["pkgname = ?"];
        let mut query_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        query_values.push(Box::new(pkg_name.to_string()));
        
        // 计算版本整数值
        if let Some(min_ver) = min_version {
            if let Ok(min_int) = VersionExp::version_to_int(min_ver) {
                conditions.push("version_int >= ?");
                query_values.push(Box::new(min_int));
            }
        }
        
        if let Some(max_ver) = max_version {
            if let Ok(max_int) = VersionExp::version_to_int(max_ver) {
                conditions.push("version_int <= ?");
                query_values.push(Box::new(max_int));
            }
        }
        
        let query = format!(
            "SELECT version, metaobjid, tag FROM pkg_versions WHERE {} ORDER BY version_int DESC",
            conditions.join(" AND ")
        );
        
        let mut stmt = conn.prepare(&query)
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        let params_slice: Vec<&dyn rusqlite::ToSql> = query_values.iter()
            .map(|v| v.as_ref() as &dyn rusqlite::ToSql)
            .collect();
        
        let versions = stmt.query_map(params_slice.as_slice(), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        let mut result = Vec::new();
        for version in versions {
            result.push(version.map_err(|e| PkgError::SqlError(e.to_string()))?);
        }
        
        Ok(result)
    }


    /// 添加包元数据
    pub fn add_pkg_meta(&self, metaobjid: &str, pkg_meta: &str, author: &str, author_pk: Option<String>) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let current_time = chrono::Utc::now().timestamp();
        let author_pk = author_pk.unwrap_or_else(|| "".to_string());
        
        conn.execute(
            "INSERT OR REPLACE INTO pkg_metas (metaobjid, pkg_meta, author, author_pk, update_time) VALUES (?, ?, ?, ?, ?)",
            params![metaobjid, pkg_meta, author, author_pk, current_time],
        ).map_err(|e| PkgError::SqlError(e.to_string()))?;

        Ok(())
    }

    /// 设置包版本的metaobjid
    pub fn set_pkg_version(&self, pkgname: &str, author: &str, version: &str, metaobjid: &str, tag: Option<&str>) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        let version_exp = VersionExp::from_str(version)?;
        if !version_exp.is_version() {
            error!("VersionExp is not a version: {} when set pkg {} version", version, pkgname);
            return Err(PkgError::ParseError(version.to_string(), "VersionExp is not a version".to_string()));
        }
        
        let current_time = Utc::now().timestamp();
        let version_int = VersionExp::version_to_int(version)?;
        
        // 检查记录是否已存在
        let exists = conn.query_row(
            "SELECT 1 FROM pkg_versions WHERE pkg_name = ? AND author = ? AND version = ?",
            params![pkgname, author, version],
            |_| Ok(true)
        ).optional().map_err(|e| PkgError::SqlError(e.to_string()))?.is_some();
        
        if exists {
            // 更新现有记录
            conn.execute(
                "UPDATE pkg_versions SET metaobjid = ?, tag = ?, update_time = ? WHERE pkg_name = ? AND author = ? AND version = ?",
                params![metaobjid, tag, current_time, pkgname, author, version],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        } else {
            // 插入新记录
            conn.execute(
                "INSERT INTO pkg_versions (pkg_name, author, version, version_int, metaobjid, tag, update_time) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![pkgname, author, version, version_int, metaobjid, tag, current_time],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        }


        Ok(())
    }

    /// 设置作者信息
    pub fn set_author_info(&self, author_name: &str, author_owner_config: Option<&str>, author_zone_config: Option<&str>) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        // 检查作者信息是否已存在
        let exists = conn.query_row(
            "SELECT 1 FROM author_info WHERE author_name = ?",
            params![author_name],
            |_| Ok(true)
        ).optional().map_err(|e| PkgError::SqlError(e.to_string()))?.is_some();
        
        // 获取当前时间戳
        let current_time = Utc::now().timestamp();
        
        if exists {
            // 如果作者信息已存在，则更新记录
            conn.execute(
                "UPDATE author_info SET author_owner_config = ?, author_zone_config = ?, update_time = ? WHERE author_name = ?",
                params![author_owner_config, author_zone_config, current_time, author_name],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        } else {
            // 如果作者信息不存在，则插入新记录
            conn.execute(
                "INSERT INTO author_info (author_name, author_owner_config, author_zone_config, update_time) VALUES (?, ?, ?, ?)",
                params![author_name, author_owner_config, author_zone_config, current_time],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
            
            return Ok(());
        }

        Ok(())
    }

    /// 根据版本要求获取包元数据
    pub fn get_pkg_meta_by_expr(&self, pkg_name: &str, author: Option<&str>, version_req: &str) -> PkgResult<Option<(String, String, String, String)>> {
        // 解析版本要求
        let req = VersionReq::parse(version_req)
            .map_err(|e| PkgError::ParseError(pkg_name.to_owned(), e.to_string()))?;
        
        // 获取所有版本
        let all_versions = self.get_all_pkg_versions(pkg_name)?;
        if all_versions.is_empty() {
            return Ok(None);
        }
        
        // 找到匹配的版本
        let mut matched_versions = Vec::new();
        for (version, metaobjid, _) in all_versions {
            if let Ok(semver) = Version::parse(&version) {
                if req.matches(&semver) {
                    matched_versions.push((version, metaobjid));
                }
            }
        }
        
        if matched_versions.is_empty() {
            return Ok(None);
        }
        
        // 选择最高的版本:
        let (_, metaobjid) = matched_versions.iter()
            .max_by(|(v1, _), (v2, _)| {
                VersionExp::compare_versions(v1, v2)
            })
            .unwrap();
        
        // 获取元数据
        let conn = Self::create_connection(&self.db_path)?;
        
        let query = match author {
            Some(auth) => {
                "SELECT m.metaobjid, m.pkg_meta, m.author, m.author_pk 
                 FROM pkg_metas m 
                 WHERE m.metaobjid = ? AND m.author = ?"
            },
            None => {
                "SELECT m.metaobjid, m.pkg_meta, m.author, m.author_pk 
                 FROM pkg_metas m 
                 WHERE m.metaobjid = ?"
            }
        };
        
        let result = if let Some(auth) = author {
            conn.query_row(
                query,
                params![metaobjid, auth],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            )
        } else {
            conn.query_row(
                query,
                params![metaobjid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            )
        }.optional().map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        Ok(result)
    }
    

    //将另一个meta_index_db中的全部记录插入当前db
    pub async fn merge_meta_index_db(&self, other_db_path: &str) -> PkgResult<()> {
        let other_db = MetaIndexDb::new(PathBuf::from(other_db_path))?;
        let mut conn = Self::create_connection(&self.db_path)?;
        let mut other_conn = Self::create_connection(&PathBuf::from(other_db_path))?;
        
        // 开始事务
        let tx = conn.transaction().map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        // 1. 合并包元数据表
        let mut pkg_metas = other_conn.prepare("SELECT metaobjid, pkg_meta, author, author_pk FROM pkg_metas")
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
            
        let pkg_meta_rows = pkg_metas.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?
            ))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        for meta_result in pkg_meta_rows {
            let (metaobjid, pkg_meta, author, author_pk) = meta_result.map_err(|e| PkgError::SqlError(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO pkg_metas (metaobjid, pkg_meta, author, author_pk) VALUES (?, ?, ?, ?)",
                params![metaobjid, pkg_meta, author, author_pk]
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        }
        
        // 2. 合并包版本表
        let mut pkg_versions = other_conn.prepare("SELECT pkg_name, version, metaobjid, tag FROM pkg_versions")
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
            
        let version_rows = pkg_versions.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?
            ))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        for version_result in version_rows {
            let (pkg_name, version, metaobjid, tag) = version_result.map_err(|e| PkgError::SqlError(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO pkg_versions (pkg_name, version, metaobjid, tag) VALUES (?, ?, ?, ?)",
                params![pkg_name, version, metaobjid, tag]
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        }
        
        // 3. 合并作者信息表
        let mut author_infos = other_conn.prepare("SELECT author_name, author_owner_config, author_zone_config FROM author_info")
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
            
        let author_rows = author_infos.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?
            ))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        for author_result in author_rows {
            let (author_name, author_owner_config, author_zone_config) = author_result.map_err(|e| PkgError::SqlError(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO author_info (author_name, author_owner_config, author_zone_config) VALUES (?, ?, ?)",
                params![author_name, author_owner_config, author_zone_config]
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        }
        
        // 提交事务
        tx.commit().map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        Ok(())
    }
}


struct MetaIndexDbList{
    dbs: Vec<PathBuf>
}

impl MetaIndexDbList{
    pub fn new(dbs: Vec<PathBuf>) -> PkgResult<Self> {
        Ok(Self { dbs })
    }

    /// 按顺序查询所有数据库，找到第一个匹配的包元数据
    pub fn get_pkg_meta(&self, pkg_name: &str, author: Option<&str>, version: Option<&str>) -> PkgResult<Option<(String, PackageMeta)>> {
        for db_path in &self.dbs {
            let db = MetaIndexDb::new(db_path.clone());
            if db.is_err() {
                continue;
            }
            let db = db.unwrap();
            let result = db.get_pkg_meta(pkg_name, author, version);
            if result.is_err() {
                continue;
            }
            let result = result.unwrap();
            if result.is_some() {
                return Ok(result);
            }
        }

        Ok(None)
    }

    /// 按顺序查询所有数据库，找到第一个匹配的作者信息
    pub fn get_author_info(&self, author_name: &str) -> PkgResult<Option<(String, Option<String>, Option<String>)>> {
        for db_path in &self.dbs {
            let db = MetaIndexDb::new(db_path.clone())?;
            if let Some(result) = db.get_author_info(author_name)? {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    /// 按顺序查询所有数据库，找到第一个匹配的包版本列表
    pub fn get_all_pkg_versions(&self, pkg_name: &str) -> PkgResult<Vec<(String, String, Option<String>)>> {
        for db_path in &self.dbs {
            let db = MetaIndexDb::new(db_path.clone())?;
            let versions = db.get_all_pkg_versions(pkg_name)?;
            if !versions.is_empty() {
                return Ok(versions);
            }
        }
        Ok(vec![])
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::cmp::Ordering;

    #[test]
    fn test_meta_db() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_meta.db");
        
        let meta_db = MetaIndexDb::new(db_path)?;
        
        // 测试添加包元数据
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg"}"#, "author1", Some("pk1".to_string()))?;
        
        // 测试设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", "3232323", Some("stable"))?;
        
        // 测试设置作者信息
        meta_db.set_author_info("author1", Some(r#"{"owner":"test"}"#), Some(r#"{"zone":"test"}"#))?;
        
        // 测试获取包元数据
        let meta = meta_db.get_pkg_meta("test-pkg", None, Some("1.0.0"))?;
        assert!(meta.is_some());
        let (metaobjid, pkg_meta) = meta.unwrap();
        assert_eq!(metaobjid, "meta1");
        //assert_eq!(pkg_meta.to, r#"{"name":"test-pkg"}"#);
        
        // 测试获取作者信息
        let author_info = meta_db.get_author_info("author1")?;
        assert!(author_info.is_some());
        let (name, owner_config, zone_config) = author_info.unwrap();
        assert_eq!(name, "author1");
        assert_eq!(owner_config.unwrap(), r#"{"owner":"test"}"#);
        assert_eq!(zone_config.unwrap(), r#"{"zone":"test"}"#);
        
        // 测试获取所有版本
        let versions = meta_db.get_all_pkg_versions("test-pkg")?;
        assert_eq!(versions.len(), 1);
        let (version, metaobjid, tag) = &versions[0];
        assert_eq!(version, "1.0.0");
        assert_eq!(metaobjid, "meta1");
        assert_eq!(tag.as_deref(), Some("stable"));
        
        Ok(())
    }




    #[test]
    fn test_version_db_operations() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_versions.db");
        
        let meta_db = MetaIndexDb::new(db_path)?;
        
        // 添加不同版本的包
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg","version":"1.0.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta2", r#"{"name":"test-pkg","version":"1.1.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta3", r#"{"name":"test-pkg","version":"1.2.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta4", r#"{"name":"test-pkg","version":"2.0.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta5", r#"{"name":"test-pkg","version":"0.9.0"}"#, "author1", Some("pk1".to_string()))?;
        
        // 设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", "3232321", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.1.0", "meta2", "3232322", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.2.0", "meta3", "3232323", Some("beta"))?;
        meta_db.set_pkg_version("test-pkg", "2.0.0", "meta4", "3232324", Some("alpha"))?;
        meta_db.set_pkg_version("test-pkg", "0.9.0", "meta5", "3232325", Some("old"))?;
        
        // 测试获取最新版本（应该是2.0.0）
        let latest = meta_db.get_pkg_meta("test-pkg", None, None)?;
        assert!(latest.is_some());
        let (metaobjid, pkg_meta) = latest.unwrap();
        assert_eq!(metaobjid, "meta4");
        //assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"2.0.0"}"#);
        
        // 测试获取特定版本
        let v1 = meta_db.get_pkg_meta("test-pkg", None, Some("1.1.0"))?;
        assert!(v1.is_some());
        let (metaobjid, pkg_meta) = v1.unwrap();
        assert_eq!(metaobjid, "meta2");
        //assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"1.1.0"}"#);
        
        // 测试获取版本范围
        let versions = meta_db.get_versions_in_range("test-pkg", Some("1.0.0"), Some("1.2.0"))?;
        assert_eq!(versions.len(), 3);
        
        // 验证版本排序是否正确（应该是降序）
        assert_eq!(versions[0].0, "1.2.0");
        assert_eq!(versions[1].0, "1.1.0");
        assert_eq!(versions[2].0, "1.0.0");
        
        // 测试按标签获取
        //let beta_version = meta_db.get_pkg_meta_by_tag("test-pkg", "beta")?;
        //assert!(beta_version.is_some());
        //let (metaobjid, _, _, _) = beta_version.unwrap();
        //assert_eq!(metaobjid, "meta3");
        
        Ok(())
    }

    
    #[test]
    fn test_version_expr_db_operations() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_version_expr.db");
        
        let meta_db = MetaIndexDb::new(db_path)?;
        
        // 添加不同版本的包
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg","version":"1.0.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta2", r#"{"name":"test-pkg","version":"1.1.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta3", r#"{"name":"test-pkg","version":"1.2.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta4", r#"{"name":"test-pkg","version":"2.0.0"}"#, "author1", Some("pk1".to_string()))?;
        meta_db.add_pkg_meta("meta5", r#"{"name":"test-pkg","version":"0.9.0"}"#, "author1", Some("pk1".to_string()))?;
        
        // 设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", "3232321", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.1.0", "meta2", "3232322", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.2.0", "meta3", "3232323", Some("beta"))?;
        meta_db.set_pkg_version("test-pkg", "2.0.0", "meta4", "3232324", Some("alpha"))?;
        meta_db.set_pkg_version("test-pkg", "0.9.0", "meta5", "3232325", Some("old"))?;
        
        // 测试使用版本表达式获取包元数据
        let test_cases = vec![
            (">1.0.0", "meta2"),  // 应该获取1.1.0版本（最低的满足条件的版本）
            (">=1.0.0", "meta1"), // 应该获取1.0.0版本（最低的满足条件的版本）
            ("<1.0.0", "meta5"),  // 应该获取0.9.0版本（最高的满足条件的版本）
            ("<=1.1.0", "meta2"), // 应该获取1.1.0版本（最高的满足条件的版本）
            ("^1.0.0", "meta3"),  // 应该获取1.2.0版本（最高的满足条件的版本）
            ("~1.1.0", "meta2"),  // 应该获取1.1.0版本（最高的满足条件的版本）
        ];
        
        for (expr, expected_meta) in test_cases {
            let meta = meta_db.get_pkg_meta_by_expr("test-pkg", None, expr)?;
            assert!(meta.is_some(), "表达式 {} 应该匹配到版本", expr);
            let (metaobjid, _, _, _) = meta.unwrap();
            assert_eq!(metaobjid, expected_meta, "表达式 {} 应该匹配到 {}", expr, expected_meta);
        }
        
        // 测试获取匹配版本列表
        // let versions = meta_db.get_versions_by_expr("test-pkg", ">1.0.0")?;
        // assert_eq!(versions.len(), 3, ">1.0.0 应该匹配3个版本");
        
        // let versions = meta_db.get_versions_by_expr("test-pkg", "^1.0.0")?;
        // assert_eq!(versions.len(), 3, "^1.0.0 应该匹配3个版本");
        
        // let versions = meta_db.get_versions_by_expr("test-pkg", "~1.0.0")?;
        // assert_eq!(versions.len(), 1, "~1.0.0 应该匹配1个版本");
        
        // 测试获取最大版本的包元数据
        let test_cases_max = vec![
            (">0.9.0", "meta4"),  // 应该获取2.0.0版本（最大的满足条件的版本）
            (">=1.0.0", "meta4"), // 应该获取2.0.0版本（最大的满足条件的版本）
            ("<2.0.0", "meta3"),  // 应该获取1.2.0版本（最大的满足条件的版本）
            ("<=1.2.0", "meta3"), // 应该获取1.2.0版本（最大的满足条件的版本）
            ("^1.0.0", "meta3"),  // 应该获取1.2.0版本（最大的满足条件的版本）
            ("~1.0.0", "meta1"),  // 应该获取1.0.0版本（最大的满足条件的版本）
        ];
        
        // for (expr, expected_meta) in test_cases_max {
        //     let meta = meta_db.get_pkg_meta_by_expr_max("test-pkg", None, expr)?;
        //     assert!(meta.is_some(), "表达式 {} 应该匹配到版本", expr);
        //     let (metaobjid, _, _, _) = meta.unwrap();
        //     assert_eq!(metaobjid, expected_meta, "表达式 {} 应该匹配到 {}", expr, expected_meta);
        // }
        
        Ok(())
    }
}