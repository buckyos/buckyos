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
    pub fn new(db_path: PathBuf,ready_only:bool) -> PkgResult<Self> {
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
                pkg_name TEXT NOT NULL,
                author TEXT ,
                version TEXT NOT NULL,
                version_int INTEGER NOT NULL,
                metaobjid TEXT NOT NULL,
                tag TEXT,
                update_time INTEGER NOT NULL,
                PRIMARY KEY (pkg_name, version)
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
            "CREATE INDEX IF NOT EXISTS idx_pkg_versions_pkgname ON pkg_versions (pkg_name)",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_metas_author ON pkg_metas (author)",
            [],
        )
        .map_err(|e| PkgError::SqlError(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pkg_versions_version_int ON pkg_versions (pkg_name, version_int)",
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


    // 关键函数，根据pkg_id获取最新版本
    //return metaobjid,pkg_meta
    pub fn get_pkg_meta(&self, pkg_id: &str) -> PkgResult<Option<(String, PackageMeta)>> {
        let package_id = PackageId::parse(pkg_id)?;
        //let conn = Self::create_connection(&self.db_path)?;
        let author = PackageId::get_author(&package_id.name.as_str());
        let version_exp = package_id.version_exp.unwrap_or_default();

        match &version_exp.version_exp {
            VersionExpType::Version(version) => {
                //返回指定版本  
                return self.get_pkg_meta_by_version(package_id.name.as_str(), author, version, version_exp.tag);
            }
            VersionExpType::Req(req) => {
                return self.get_pkg_meta_by_version_expr(package_id.name.as_str(), author, req, version_exp.tag);
            }
            VersionExpType::None => {
                let req = VersionReq::parse("*").unwrap();
                return self.get_pkg_meta_by_version_expr(package_id.name.as_str(), author, &req, version_exp.tag);
            }
        }        
    }

    pub fn get_pkg_meta_by_version(&self,pkg_name: &str,author: Option<String>,version: &Version,tag: Option<String>) -> PkgResult<Option<(String, PackageMeta)>> {
        let conn = Self::create_connection(&self.db_path)?;
        // 构建查询条件
        let mut query = String::from(
            "SELECT pv.metaobjid, pm.pkg_meta FROM pkg_versions pv 
            JOIN pkg_metas pm ON pv.metaobjid = pm.metaobjid 
            WHERE pv.pkg_name = ? AND pv.version = ?"
        );
        
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(pkg_name.to_string()),
            Box::new(version.to_string())
        ];
        
        // 如果有作者信息，添加作者条件
        if let Some(author_value) = author {
            query.push_str(" AND pv.author = ?");
            params_vec.push(Box::new(author_value));
        }
        
        // 如果有标签信息，添加标签条件
        if let Some(tag_value) = tag {
            query.push_str(" AND pv.tag = ?");
            params_vec.push(Box::new(tag_value));
        }
        
        // 准备查询语句
        let mut stmt = conn.prepare(&query)
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        // 转换参数为引用切片
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|p| p.as_ref())
            .collect();
        
        // 执行查询
        let result = stmt.query_row(params_slice.as_slice(), |row| {
            let metaobjid: String = row.get(0)?;
            let pkg_meta_str: String = row.get(1)?;
            let pkg_meta = serde_json::from_str(&pkg_meta_str);
            if pkg_meta.is_err() {
                let err_str = pkg_meta.err().unwrap().to_string();
                error!("parse pkg_meta_str failed: {:?}", err_str);
                return Err(rusqlite::Error::InvalidColumnType(0, err_str, rusqlite::types::Type::Text));
            }
            
            Ok((metaobjid, pkg_meta.unwrap()))
        });
        
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(PkgError::SqlError(e.to_string()))
        }
    }

    pub fn get_pkg_meta_by_version_expr(&self,pkg_name: &str,author: Option<String>,version_req:&VersionReq,tag: Option<String>) -> PkgResult<Option<(String, PackageMeta)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        // 尝试获取版本范围
        let version_exp = VersionExp {
            tag: None,
            version_exp: VersionExpType::Req(version_req.clone())
        };
        
        let versions = match version_exp.to_range_int() {
            Ok((min_version, max_version)) => {
                // 如果能获取版本范围，使用数据库查询加速
                Self::get_versions_in_range(&conn, pkg_name, min_version, max_version, tag.as_deref())?
            },
            Err(_) => {
                // 如果不能获取范围，获取所有版本
                let mut query = String::from(
                    "SELECT version, metaobjid, tag FROM pkg_versions WHERE pkg_name = ?"
                );
                
                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
                    Box::new(pkg_name.to_string())
                ];
                
                if let Some(author_value) = &author {
                    query.push_str(" AND author = ?");
                    params_vec.push(Box::new(author_value.clone()));
                }
                
                if let Some(tag_value) = &tag {
                    query.push_str(" AND tag = ?");
                    params_vec.push(Box::new(tag_value.clone()));
                }
                
                query.push_str(" ORDER BY version_int DESC");
                
                let mut stmt = conn.prepare(&query)
                    .map_err(|e| PkgError::SqlError(e.to_string()))?;
                
                let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec
                    .iter()
                    .map(|p| p.as_ref())
                    .collect();
                
                let rows = stmt.query_map(params_slice.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?
                    ))
                }).map_err(|e| PkgError::SqlError(e.to_string()))?;
                
                let mut result = Vec::new();
                for row in rows {
                    result.push(row.map_err(|e| PkgError::SqlError(e.to_string()))?);
                }
                result
            }
        };

        // 如果没有找到任何版本，直接返回None
        if versions.is_empty() {
            return Ok(None);
        }

        // 找到符合版本要求的最新版本
        let mut latest_version: Option<(String, String)> = None;
        for (version, metaobjid, _) in versions {
            // 解析版本号
            if let Ok(v) = Version::parse(&version) {
                // 检查版本是否满足要求
                if version_req.matches(&v) {
                    match &latest_version {
                        None => {
                            latest_version = Some((version.clone(), metaobjid.clone()));
                        }
                        Some((latest_v, _)) => {
                            // 比较版本号，保留较新的版本
                            if VersionExp::compare_versions(&version, latest_v) == std::cmp::Ordering::Greater {
                                latest_version = Some((version.clone(), metaobjid.clone()));
                            }
                        }
                    }
                }
            }
        }

        // 如果找到了符合要求的版本，获取其元数据
        if let Some((_, metaobjid)) = latest_version {
            let pkg_meta: String = conn.query_row(
                "SELECT pkg_meta FROM pkg_metas WHERE metaobjid = ?",
                params![metaobjid],
                |row| row.get(0)
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;

            let pkg_meta: PackageMeta = serde_json::from_str(&pkg_meta)
                .map_err(|e| PkgError::ParseError(pkg_meta.clone(), e.to_string()))?;

            Ok(Some((metaobjid, pkg_meta)))
        } else {
            Ok(None)
        }
    }

    /// 获取meta_index_db中，指定pkg_name的所有版本
    pub fn list_all_pkg_versions(&self, pkg_name: &str) -> PkgResult<Vec<(String, String, Option<String>)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let mut stmt = conn.prepare(
            "SELECT version, metaobjid, tag FROM pkg_versions WHERE pkg_name = ? ORDER BY update_time DESC"
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

    /// 获取版本范围,如果有tag，则只返回有tag的版本
    fn get_versions_in_range(conn:&Connection, pkg_name: &str, min_version:u64, max_version:u64,tag:Option<&str>) -> PkgResult<Vec<(String, String, Option<String>)>> {
        let mut query = String::from(
            "SELECT version, metaobjid, tag FROM pkg_versions 
            WHERE pkg_name = ? AND version_int >= ? AND version_int <= ?"
        );
        
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(pkg_name.to_string()),
            Box::new(min_version),
            Box::new(max_version)
        ];
        
        if let Some(tag_value) = tag {
            query.push_str(" AND tag = ?");
            params_vec.push(Box::new(tag_value.to_string()));
        }
        
        query.push_str(" ORDER BY version_int DESC");
        
        let mut stmt = conn.prepare(&query)
            .map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|p| p.as_ref())
            .collect();
        
        let versions = stmt.query_map(params_slice.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?
            ))
        }).map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        let mut result = Vec::new();
        for version in versions {
            result.push(version.map_err(|e| PkgError::SqlError(e.to_string()))?);
        }
    
        Ok(result)
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

    pub fn add_pkg_meta_batch(&self, pkg_meta_map: &HashMap<String,PackageMetaNode>) -> PkgResult<()> {
        let mut conn: Connection = Self::create_connection(&self.db_path)?;
        // 开始事务
        let tx = conn.transaction().map_err(|e| PkgError::SqlError(e.to_string()))?;
        
        let current_time = chrono::Utc::now().timestamp();
        
        for (metaobjid, meta_node) in pkg_meta_map {
            // 插入包元数据
            tx.execute(
                "INSERT OR REPLACE INTO pkg_metas (metaobjid, pkg_meta, author, author_pk, update_time) VALUES (?, ?, ?, ?, ?)",
                params![metaobjid, meta_node.meta_jwt, meta_node.author, meta_node.author_pk, current_time],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;

            // 设置包版本
            let version_int = VersionExp::version_to_int(&meta_node.version)?;
            
            // 检查记录是否已存在
            let exists = tx.query_row(
                "SELECT 1 FROM pkg_versions WHERE pkg_name = ? AND author = ? AND version = ?",
                params![meta_node.pkg_name, meta_node.author, meta_node.version],
                |_| Ok(true)
            ).optional().map_err(|e| PkgError::SqlError(e.to_string()))?.is_some();
            
            if exists {
                // 更新现有记录
                tx.execute(
                    "UPDATE pkg_versions SET metaobjid = ?, tag = ?, update_time = ? WHERE pkg_name = ? AND author = ? AND version = ?",
                    params![metaobjid, meta_node.tag, current_time, meta_node.pkg_name, meta_node.author, meta_node.version],
                ).map_err(|e| PkgError::SqlError(e.to_string()))?;
            } else {
                // 插入新记录
                tx.execute(
                    "INSERT INTO pkg_versions (pkg_name, author, version, version_int, metaobjid, tag, update_time) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![meta_node.pkg_name, meta_node.author, meta_node.version, version_int, metaobjid, meta_node.tag, current_time],
                ).map_err(|e| PkgError::SqlError(e.to_string()))?;
            }
        }
        
        // 提交事务
        tx.commit().map_err(|e| PkgError::SqlError(e.to_string()))?;
        
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
                "UPDATE author_info SET author_owner_config = ?, author_zone_config = ?  WHERE author_name = ?",
                params![author_owner_config, author_zone_config, author_name],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
        } else {
            // 如果作者信息不存在，则插入新记录
            conn.execute(
                "INSERT INTO author_info (author_name, author_owner_config, author_zone_config) VALUES (?, ?, ?)",
                params![author_name, author_owner_config, author_zone_config],
            ).map_err(|e| PkgError::SqlError(e.to_string()))?;
            
            return Ok(());
        }

        Ok(())
    }



    //将另一个meta_index_db中的全部记录插入当前db
    pub async fn merge_meta_index_db(&self, other_db_path: &str) -> PkgResult<()> {
        let other_db = MetaIndexDb::new(PathBuf::from(other_db_path),true)?;
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

    pub fn get_pkg_meta(&self, pkg_id: &str) -> PkgResult<Option<(String, PackageMeta)>> {
        for db_path in &self.dbs {
            let db = MetaIndexDb::new(db_path.clone(),true);
            if db.is_err() {
                continue;
            }
            let db = db.unwrap();   
            let pkg_meta = db.get_pkg_meta(pkg_id);
            if pkg_meta.is_err() {
                continue;
            }
            let pkg_meta = pkg_meta.unwrap();
            if pkg_meta.is_some() {
                return Ok(pkg_meta);
            }       
        }
        Ok(None)
    }

    /// 按顺序查询所有数据库，找到第一个匹配的作者信息
    pub fn get_author_info(&self, author_name: &str) -> PkgResult<Option<(String, Option<String>, Option<String>)>> {
        for db_path in &self.dbs {
            let db = MetaIndexDb::new(db_path.clone(),true).unwrap();
            if let Some(result) = db.get_author_info(author_name)? {
                return Ok(Some(result));
            }
        }
        Ok(None)
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
        
        let meta_db = MetaIndexDb::new(db_path,false)?;
        let test_pkg_meta = PackageMeta {
            pkg_name: "test-pkg".to_string(),
            version: "1.0.1".to_string(),
            author: "author1".to_string(),
            tag: Some("stable".to_string()),
            category: Some("app".to_string()),
            chunk_id: Some("chunk1".to_string()),
            chunk_size: Some(100),
            chunk_url: Some("http://test.com/chunk1".to_string()),
            deps: HashMap::new(),
            pub_time: Utc::now().timestamp(),
        };

        let test_pkg_meta_str = serde_json::to_string(&test_pkg_meta).unwrap();
        
        // 测试添加包元数据
        meta_db.add_pkg_meta("meta1", &test_pkg_meta_str, "author1", Some("pk1".to_string()))?;
        
        // 测试设置包版本
        meta_db.set_pkg_version("test-pkg", "author1", "1.0.1", "meta1", Some("stable"))?;
        
        // 测试设置作者信息
        meta_db.set_author_info("author1", Some(r#"{"owner":"test"}"#), Some(r#"{"zone":"test"}"#))?;
        
        // 测试获取包元数据
        let meta = meta_db.get_pkg_meta("test-pkg#1.0.1:stable")?;
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
        let versions = meta_db.list_all_pkg_versions("test-pkg")?;
        assert_eq!(versions.len(), 1);
        let (version, metaobjid, tag) = &versions[0];
        assert_eq!(version, "1.0.1");
        assert_eq!(metaobjid, "meta1");
        assert_eq!(tag.as_deref(), Some("stable"));
        
        Ok(())
    }




    #[test]
    fn test_version_db_operations() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_versions.db");
        
        let meta_db = MetaIndexDb::new(db_path,false)?;
        let mut test_pkg_meta1 = PackageMeta {
            pkg_name: "test-pkg".to_string(),
            version: "1.0.0".to_string(),
            author: "author1".to_string(),
            tag: Some("stable".to_string()),
            category: Some("app".to_string()),
            chunk_id: Some("chunk1".to_string()),
            chunk_size: Some(100),
            chunk_url: Some("http://test.com/chunk1".to_string()),
            deps: HashMap::new(),
            pub_time: Utc::now().timestamp(),
        };  
        let test_pkg_meta_str1 = serde_json::to_string(&test_pkg_meta1).unwrap();
        meta_db.add_pkg_meta("meta1", &test_pkg_meta_str1, "author1", Some("pk1".to_string()))?;
        let mut test_pkg_meta2 = test_pkg_meta1.clone();
        test_pkg_meta2.version = "1.1.0".to_string();
        let test_pkg_meta_str2 = serde_json::to_string(&test_pkg_meta2).unwrap();
        meta_db.add_pkg_meta("meta2", &test_pkg_meta_str2, "author1", Some("pk1".to_string()))?;
        let mut test_pkg_meta3 = test_pkg_meta1.clone();
        test_pkg_meta3.version = "1.2.0".to_string();
        let test_pkg_meta_str3 = serde_json::to_string(&test_pkg_meta3).unwrap();
        meta_db.add_pkg_meta("meta3", &test_pkg_meta_str3, "author1", Some("pk1".to_string()))?;
        let mut test_pkg_meta4 = test_pkg_meta1.clone();
        test_pkg_meta4.version = "2.0.0".to_string();
        let test_pkg_meta_str4 = serde_json::to_string(&test_pkg_meta4).unwrap();
        meta_db.add_pkg_meta("meta4", &test_pkg_meta_str4, "author1", Some("pk1".to_string()))?;
        let mut test_pkg_meta5 = test_pkg_meta1.clone();
        test_pkg_meta5.version = "0.9.0".to_string();
        let test_pkg_meta_str5 = serde_json::to_string(&test_pkg_meta5).unwrap();
        meta_db.add_pkg_meta("meta5", &test_pkg_meta_str5, "author1", Some("pk1".to_string()))?;
        
        // 设置包版本
        meta_db.set_pkg_version("test-pkg", "author1", "1.0.0", "meta1", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "author1", "1.1.0", "meta2", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "author1", "1.2.0", "meta3", Some("beta"))?;
        meta_db.set_pkg_version("test-pkg", "author1", "2.0.0", "meta4", Some("alpha"))?;
        meta_db.set_pkg_version("test-pkg", "author1", "0.9.0", "meta5", Some("old"))?;
        
        // 测试获取最新版本（应该是2.0.0）
        let latest = meta_db.get_pkg_meta("test-pkg#*")?;
        assert!(latest.is_some());
        let (metaobjid, pkg_meta) = latest.unwrap();
        assert_eq!(metaobjid, "meta4");
        //assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"2.0.0"}"#);
        
        // 测试获取特定版本
        let v1 = meta_db.get_pkg_meta("test-pkg#1.1.0:stable")?;
        assert!(v1.is_some());
        let (metaobjid, pkg_meta) = v1.unwrap();
        assert_eq!(metaobjid, "meta2");
        //assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"1.1.0"}"#);
        
        // 测试获取版本范围
        let v1 = meta_db.get_pkg_meta("test-pkg#>=1.0.0, <=1.2.0")?;
        assert!(v1.is_some());
        let (metaobjid, pkg_meta) = v1.unwrap();
        assert_eq!(metaobjid, "meta3");
        
        // 测试按标签获取
        let beta_version = meta_db.get_pkg_meta("test-pkg#*:beta")?;
        assert!(beta_version.is_some());
        let (metaobjid, pkg_meta) = beta_version.unwrap();
        assert_eq!(metaobjid, "meta3");
        
        Ok(())
    }

}