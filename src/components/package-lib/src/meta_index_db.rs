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
use chrono::Utc;
use crate::error::*;
use crate::meta::*;
use semver;
use std::str::FromStr;
use crate::meta::*;

// 版本表达式操作符
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum VersionOp {
    Eq,  // =
    Gt,  // >
    Lt,  // <
    Gte, // >=
    Lte, // <=
    Caret, // ^
    Tilde, // ~
}

// 版本表达式
#[derive(Debug, Clone)]
pub struct VersionExpr {
    pub op: VersionOp,
    pub version: String,
}

impl VersionExpr {
    // 解析版本表达式字符串
    pub fn parse(expr: &str) -> PkgResult<Self> {
        let expr = expr.trim();
        
        if expr.starts_with(">=") {
            Ok(VersionExpr {
                op: VersionOp::Gte,
                version: expr[2..].trim().to_string(),
            })
        } else if expr.starts_with(">") {
            Ok(VersionExpr {
                op: VersionOp::Gt,
                version: expr[1..].trim().to_string(),
            })
        } else if expr.starts_with("<=") {
            Ok(VersionExpr {
                op: VersionOp::Lte,
                version: expr[2..].trim().to_string(),
            })
        } else if expr.starts_with("<") {
            Ok(VersionExpr {
                op: VersionOp::Lt,
                version: expr[1..].trim().to_string(),
            })
        } else if expr.starts_with("^") {
            Ok(VersionExpr {
                op: VersionOp::Caret,
                version: expr[1..].trim().to_string(),
            })
        } else if expr.starts_with("~") {
            Ok(VersionExpr {
                op: VersionOp::Tilde,
                version: expr[1..].trim().to_string(),
            })
        } else if expr.starts_with("=") {
            Ok(VersionExpr {
                op: VersionOp::Eq,
                version: expr[1..].trim().to_string(),
            })
        } else {
            // 默认为等于操作符
            Ok(VersionExpr {
                op: VersionOp::Eq,
                version: expr.to_string(),
            })
        }
    }
    
    // 检查版本是否满足表达式
    pub fn matches(&self, version: &str) -> bool {
        match self.op {
            VersionOp::Eq => {
                // 等于操作符，直接比较
                MetaIndexDb::compare_versions(version, &self.version) == std::cmp::Ordering::Equal
            },
            VersionOp::Gt => {
                // 大于操作符
                MetaIndexDb::compare_versions(version, &self.version) == std::cmp::Ordering::Greater
            },
            VersionOp::Lt => {
                // 小于操作符
                MetaIndexDb::compare_versions(version, &self.version) == std::cmp::Ordering::Less
            },
            VersionOp::Gte => {
                // 大于等于操作符
                let cmp = MetaIndexDb::compare_versions(version, &self.version);
                cmp == std::cmp::Ordering::Greater || cmp == std::cmp::Ordering::Equal
            },
            VersionOp::Lte => {
                // 小于等于操作符
                let cmp = MetaIndexDb::compare_versions(version, &self.version);
                cmp == std::cmp::Ordering::Less || cmp == std::cmp::Ordering::Equal
            },
            VersionOp::Caret => {
                // ^ 操作符，兼容版本（允许次版本号和修订版本号变化）
                // 例如 ^1.2.3 匹配 1.2.3 到 <2.0.0
                if let Ok(req_ver) = semver::Version::parse(&self.version) {
                    if let Ok(check_ver) = semver::Version::parse(version) {
                        // 主版本号必须相同
                        if req_ver.major != check_ver.major {
                            return false;
                        }
                        
                        // 如果主版本号为0，则次版本号也必须相同
                        if req_ver.major == 0 && req_ver.minor != check_ver.minor {
                            return false;
                        }
                        
                        // 版本必须大于等于指定版本
                        return check_ver >= req_ver;
                    }
                }
                
                // 非标准版本格式，使用自定义逻辑
                let parts: Vec<&str> = self.version.split('.').collect();
                let check_parts: Vec<&str> = version.split('.').collect();
                
                // 主版本号必须相同
                if parts.get(0) != check_parts.get(0) {
                    return false;
                }
                
                // 如果主版本号为0，则次版本号也必须相同
                if parts.get(0) == Some(&"0") && parts.get(1) != check_parts.get(1) {
                    return false;
                }
                
                // 版本必须大于等于指定版本
                MetaIndexDb::compare_versions(version, &self.version) != std::cmp::Ordering::Less
            },
            VersionOp::Tilde => {
                // ~ 操作符，允许修订版本号变化
                // 例如 ~1.2.3 匹配 1.2.3 到 <1.3.0
                if let Ok(req_ver) = semver::Version::parse(&self.version) {
                    if let Ok(check_ver) = semver::Version::parse(version) {
                        // 主版本号和次版本号必须相同
                        if req_ver.major != check_ver.major || req_ver.minor != check_ver.minor {
                            return false;
                        }
                        
                        // 版本必须大于等于指定版本
                        return check_ver >= req_ver;
                    }
                }
                
                // 非标准版本格式，使用自定义逻辑
                let parts: Vec<&str> = self.version.split('.').collect();
                let check_parts: Vec<&str> = version.split('.').collect();
                
                // 主版本号和次版本号必须相同
                if parts.get(0) != check_parts.get(0) || parts.get(1) != check_parts.get(1) {
                    return false;
                }
                
                // 版本必须大于等于指定版本
                MetaIndexDb::compare_versions(version, &self.version) != std::cmp::Ordering::Less
            }
        }
    }
}

impl FromStr for VersionExpr {
    type Err = PkgError;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        VersionExpr::parse(s)
    }
}

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
    pub fn get_pkg_meta(&self, pkg_name: &str, author: Option<&str>, version: Option<&str>) -> PkgResult<Option<(String, PackageMeta)>> {
        // let conn = Self::create_connection(&self.db_path)?;
        
        // let query = match (version, author) {
        //     (Some(_), Some(_)) => "SELECT m.metaobjid, m.pkg_meta FROM pkg_versions v JOIN pkg_metas m ON v.metaobjid = m.metaobjid WHERE v.pkgname = ? AND v.version = ? AND m.author = ?",
        //     (Some(_), None) => "SELECT m.metaobjid, m.pkg_meta FROM pkg_versions v JOIN pkg_metas m ON v.metaobjid = m.metaobjid WHERE v.pkgname = ? AND v.version = ?",
        //     (None, Some(_)) => "SELECT m.metaobjid, m.pkg_meta FROM pkg_versions v JOIN pkg_metas m ON v.metaobjid = m.metaobjid WHERE v.pkgname = ? AND m.author = ? ORDER BY v.version_int DESC LIMIT 1",
        //     (None, None) => "SELECT m.metaobjid, m.pkg_meta FROM pkg_versions v JOIN pkg_metas m ON v.metaobjid = m.metaobjid WHERE v.pkgname = ? ORDER BY v.version_int DESC LIMIT 1",
        // };

        // let result = match (version, author) {
        //     (Some(ver), Some(auth)) => conn.query_row(query, params![pkg_name, ver, auth], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(|e| PkgError::SqlError(e.to_string()))?,
        //     (Some(ver), None) => conn.query_row(query, params![pkg_name, ver], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(|e| PkgError::SqlError(e.to_string()))?,
        //     (None, Some(auth)) => conn.query_row(query, params![pkg_name, auth], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?,
        //     (None, None) => conn.query_row(query, params![pkg_name], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?,
        // };

        // Ok(result.map(|(metaobjid, pkg_meta_str)| {
        //     let pkg_meta = serde_json::from_str(&pkg_meta_str).unwrap_or_else(|e| {
        //         warn!("解析包元数据失败: {}", e);
        //         return Err(PkgError::ParseError(
        //             pkg_name.to_owned(),
        //             "Package metadata parse error".to_owned(),
        //         ))
        //     });
        //     (metaobjid, pkg_meta)
        // }))

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
            if let Ok(min_int) = MetaIndexDb::version_to_int(min_ver) {
                conditions.push("version_int >= ?");
                query_values.push(Box::new(min_int));
            }
        }
        
        if let Some(max_ver) = max_version {
            if let Ok(max_int) = MetaIndexDb::version_to_int(max_ver) {
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

    /// 获取按标签查询的包元数据
    pub fn get_pkg_meta_by_tag(&self, pkg_name: &str, tag: &str) -> PkgResult<Option<(String, String, String, String)>> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let query = "SELECT m.metaobjid, m.pkg_meta, m.author, m.author_pk 
                     FROM pkg_versions v 
                     JOIN pkg_metas m ON v.metaobjid = m.metaobjid 
                     WHERE v.pkgname = ? AND v.tag = ?";
        
        conn.query_row(
            query,
            params![pkg_name, tag],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        ).optional().map_err(|e| PkgError::SqlError(e.to_string()))
    }

    // 修改接口

    /// 添加包元数据
    pub fn add_pkg_meta(&self, metaobjid: &str, pkg_meta: &str, author: &str, author_pk: &str) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let current_time = chrono::Utc::now().timestamp();
        
        conn.execute(
            "INSERT OR REPLACE INTO pkg_metas (metaobjid, pkg_meta, author, author_pk, update_time) VALUES (?, ?, ?, ?, ?)",
            params![metaobjid, pkg_meta, author, author_pk, current_time],
        ).map_err(|e| PkgError::SqlError(e.to_string()))?;

        Ok(())
    }

    /// 设置包版本
    pub fn set_pkg_version(&self, pkgname: &str, version: &str, metaobjid: &str, tag: Option<&str>) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        
        let current_time = Utc::now().timestamp();
        let version_int = MetaIndexDb::version_to_int(version)?;
        
        conn.execute(
            "INSERT OR REPLACE INTO pkg_versions (pkgname, version, version_int, metaobjid, tag, update_time) 
             VALUES (?, ?, ?, ?, ?, ?)",
            params![pkgname, version, version_int, metaobjid, tag, current_time],
        ).map_err(|e| PkgError::SqlError(e.to_string()))?;

        Ok(())
    }

    /// 设置作者信息
    pub fn set_author_info(&self, author_name: &str, author_owner_config: Option<&str>, author_zone_config: Option<&str>) -> PkgResult<()> {
        let conn = Self::create_connection(&self.db_path)?;
        
        conn.execute(
            "INSERT OR REPLACE INTO author_info (author_name, author_owner_config, author_zone_config) VALUES (?, ?, ?)",
            params![author_name, author_owner_config, author_zone_config],
        ).map_err(|e| PkgError::SqlError(e.to_string()))?;

        Ok(())
    }

    // 将版本号转换为整数表示
    pub fn version_to_int(version: &str) -> PkgResult<i64> {
        let parts: Vec<&str> = version.split('.').collect();
        
        // 基本格式检查
        if parts.len() < 1 || parts.len() > 4 {
            return Err(PkgError::VersionError(format!("无效的版本格式: {}", version)));
        }
        
        // 解析各部分
        let major = parts.get(0).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let build = parts.get(3).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        
        // 将各部分组合成一个整数
        // 每部分使用16位 (0xFFFF)
        let version_int = (major << 48) | (minor << 32) | (patch << 16) | build;
        
        Ok(version_int)
    }

    fn parse_pkg_name(full_name: &str) -> (String, Option<String>) {
        if let Some(pos) = full_name.find('/') {
            let author = &full_name[0..pos];
            let pkg_name = &full_name[pos+1..];
            (pkg_name.to_string(), Some(author.to_string()))
        } else {
            (full_name.to_string(), None)
        }
    }

    // pub fn get_pkg_meta_with_full_name(&self, full_name: &str, version: Option<&str>) -> PkgResult<Option<(String, String, String, String)>> {
    //     let (pkg_name, author) = MetaIndexDb::parse_pkg_name(full_name);
    //     let author_ref = author.as_deref();
        
    //     self.get_pkg_meta(&pkg_name, author_ref, version)
    // }

    // 使用 semver 库比较版本
    pub fn compare_versions(v1: &str, v2: &str) -> std::cmp::Ordering {
        match (semver::Version::parse(v1), semver::Version::parse(v2)) {
            (Ok(v1), Ok(v2)) => v1.cmp(&v2),
            // 处理非标准版本格式的情况
            _ => {
                // 自定义比较逻辑，使用我们的整数表示进行比较
                match (Self::version_to_int(v1), Self::version_to_int(v2)) {
                    (Ok(v1_int), Ok(v2_int)) => v1_int.cmp(&v2_int),
                    // 如果转换失败，则按字符串比较
                    _ => v1.cmp(v2)
                }
            }
        }
    }

    // 从整数表示转回版本号字符串
    pub fn int_to_version(version_int: i64) -> String {
        let major = (version_int >> 48) & 0xFFFF;
        let minor = (version_int >> 32) & 0xFFFF;
        let patch = (version_int >> 16) & 0xFFFF;
        let build = version_int & 0xFFFF;
        
        // 如果后面的部分为0，则不显示
        if build == 0 {
            if patch == 0 {
                if minor == 0 {
                    return format!("{}", major);
                }
                return format!("{}.{}", major, minor);
            }
            return format!("{}.{}.{}", major, minor, patch);
        }
        
        format!("{}.{}.{}.{}", major, minor, patch, build)
    }

    /// 根据版本表达式获取包元数据
    pub fn get_pkg_meta_by_expr(&self, pkg_name: &str, author: Option<&str>, version_expr: &str) -> PkgResult<Option<(String, String, String, String)>> {
        // 解析版本表达式
        let expr = VersionExpr::parse(version_expr)?;
        
        // 获取所有版本
        let all_versions = self.get_all_pkg_versions(pkg_name)?;
        if all_versions.is_empty() {
            return Ok(None);
        }
        
        // 找到匹配的版本
        let mut matched_versions = Vec::new();
        for (version, metaobjid, _) in all_versions {
            if expr.matches(&version) {
                matched_versions.push((version, metaobjid));
            }
        }
        
        if matched_versions.is_empty() {
            return Ok(None);
        }
        
        // 根据操作符选择合适的版本
        let (_, metaobjid) = match expr.op {
            // 对于 > 和 >= 操作符，选择最低的匹配版本
            VersionOp::Gt | VersionOp::Gte => {
                matched_versions.iter()
                    .min_by(|(v1, _), (v2, _)| Self::compare_versions(v1, v2))
                    .unwrap()
            },
            // 对于 < 和 <= 操作符，选择最高的匹配版本
            VersionOp::Lt | VersionOp::Lte => {
                matched_versions.iter()
                    .max_by(|(v1, _), (v2, _)| Self::compare_versions(v1, v2))
                    .unwrap()
            },
            // 对于其他操作符，选择最高的匹配版本
            _ => {
                matched_versions.iter()
                    .max_by(|(v1, _), (v2, _)| Self::compare_versions(v1, v2))
                    .unwrap()
            }
        };
        
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
    
    /// 根据版本表达式获取包版本列表
    pub fn get_versions_by_expr(&self, pkg_name: &str, version_expr: &str) -> PkgResult<Vec<(String, String, Option<String>)>> {
        // 解析版本表达式
        let expr = VersionExpr::parse(version_expr)?;
        
        // 获取所有版本
        let all_versions = self.get_all_pkg_versions(pkg_name)?;
        
        // 过滤出匹配的版本
        let mut matched_versions = Vec::new();
        for version_info in all_versions {
            if expr.matches(&version_info.0) {
                matched_versions.push(version_info);
            }
        }
        
        Ok(matched_versions)
    }
    
    /// 根据版本表达式获取最大版本的包元数据
    pub fn get_pkg_meta_by_expr_max(&self, pkg_name: &str, author: Option<&str>, version_expr: &str) -> PkgResult<Option<(String, String, String, String)>> {
        // 解析版本表达式
        let expr = VersionExpr::parse(version_expr)?;
        
        // 获取所有版本
        let all_versions = self.get_all_pkg_versions(pkg_name)?;
        if all_versions.is_empty() {
            return Ok(None);
        }
        
        // 找到匹配的版本
        let mut matched_versions = Vec::new();
        for (version, metaobjid, _) in all_versions {
            if expr.matches(&version) {
                matched_versions.push((version, metaobjid));
            }
        }
        
        if matched_versions.is_empty() {
            return Ok(None);
        }
        
        // 选择版本号最大的那个
        let (_, metaobjid) = matched_versions.iter()
            .max_by(|(v1, _), (v2, _)| Self::compare_versions(v1, v2))
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
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg"}"#, "author1", "pk1")?;
        
        // 测试设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", Some("stable"))?;
        
        // 测试设置作者信息
        meta_db.set_author_info("author1", Some(r#"{"owner":"test"}"#), Some(r#"{"zone":"test"}"#))?;
        
        // 测试获取包元数据
        let meta = meta_db.get_pkg_meta("test-pkg", None, Some("1.0.0"))?;
        assert!(meta.is_some());
        let (metaobjid, pkg_meta) = meta.unwrap();
        assert_eq!(metaobjid, "meta1");
        assert_eq!(pkg_meta, r#"{"name":"test-pkg"}"#);
        
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
    fn test_version_to_int() -> PkgResult<()> {
        // 测试版本号转整数
        let test_cases = vec![
            ("1", 0x0001_0000_0000_0000),
            ("1.0", 0x0001_0000_0000_0000),
            ("1.2", 0x0001_0002_0000_0000),
            ("1.2.3", 0x0001_0002_0003_0000),
            ("1.2.3.4", 0x0001_0002_0003_0004),
            ("10.20.30.40", 0x000A_0014_001E_0028),
            ("0.0.0.0", 0x0000_0000_0000_0000),
            // 最大值测试 - 使用i64范围内的最大值
            ("32767.65535.65535.65535", 0x7FFF_FFFF_FFFF_FFFF),
        ];

        for (version, expected) in &test_cases {
            let result = MetaIndexDb::version_to_int(version)?;
            assert_eq!(result, *expected, "版本 {} 转换为整数应该是 {:#X}, 但得到了 {:#X}", version, expected, result);
        }

        // 测试整数转版本号
        let int_to_version_test_cases = vec![
            (0x0001_0000_0000_0000, "1"),
            (0x0001_0002_0000_0000, "1.2"),
            (0x0001_0002_0003_0000, "1.2.3"),
            (0x0001_0002_0003_0004, "1.2.3.4"),
            (0x000A_0014_001E_0028, "10.20.30.40"),
            (0x0000_0000_0000_0000, "0"),
            (0x7FFF_FFFF_FFFF_FFFF, "32767.65535.65535.65535"),
        ];

        for (version_int, expected) in &int_to_version_test_cases {
            let result = MetaIndexDb::int_to_version(*version_int);
            assert_eq!(result, *expected, "整数 {:#X} 转换为版本号应该是 {}, 但得到了 {}", version_int, expected, result);
        }

        Ok(())
    }

    #[test]
    fn test_version_comparison() -> PkgResult<()> {
        // 测试标准semver格式的版本比较
        let semver_test_cases = vec![
            ("1.0.0", "1.0.0", Ordering::Equal),
            ("1.0.0", "1.0.1", Ordering::Less),
            ("1.0.1", "1.0.0", Ordering::Greater),
            ("1.0.0", "1.1.0", Ordering::Less),
            ("1.1.0", "1.0.0", Ordering::Greater),
            ("1.0.0", "2.0.0", Ordering::Less),
            ("2.0.0", "1.0.0", Ordering::Greater),
            ("1.0.0-alpha", "1.0.0", Ordering::Less),
            ("1.0.0", "1.0.0-alpha", Ordering::Greater),
            ("1.0.0-alpha", "1.0.0-beta", Ordering::Less),
            ("1.0.0-beta", "1.0.0-alpha", Ordering::Greater),
            ("1.0.0-beta", "1.0.0-alpha+323ad", Ordering::Greater),
        ];

        for (v1, v2, expected) in semver_test_cases {
            let result = MetaIndexDb::compare_versions(v1, v2);
            assert_eq!(result, expected, "比较 {} 和 {} 应该得到 {:?}, 但得到了 {:?}", v1, v2, expected, result);
        }

        // 测试非标准格式的版本比较（使用我们的自定义逻辑）
        let custom_test_cases = vec![
            ("1", "1", Ordering::Equal),
            ("1", "1.0", Ordering::Equal),
            ("1.0", "1.0.0", Ordering::Equal),
            ("1", "2", Ordering::Less),
            ("2", "1", Ordering::Greater),
            ("1.2", "1.3", Ordering::Less),
            ("1.3", "1.2", Ordering::Greater),
            ("1.2.3", "1.2.4", Ordering::Less),
            ("1.2.4", "1.2.3", Ordering::Greater),
            ("1.2.3.4", "1.2.3.5", Ordering::Less),
            ("1.2.3.5", "1.2.3.4", Ordering::Greater),
            ("1.2.3", "1.2.3.0", Ordering::Equal),
            ("1.2.0", "1.2", Ordering::Equal),
            ("1.0.0", "1", Ordering::Equal),
        ];

        for (v1, v2, expected) in custom_test_cases {
            let result = MetaIndexDb::compare_versions(v1, v2);
            assert_eq!(result, expected, "比较 {} 和 {} 应该得到 {:?}, 但得到了 {:?}", v1, v2, expected, result);
        }

        Ok(())
    }

    #[test]
    fn test_version_db_operations() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_versions.db");
        
        let meta_db = MetaIndexDb::new(db_path)?;
        
        // 添加不同版本的包
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg","version":"1.0.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta2", r#"{"name":"test-pkg","version":"1.1.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta3", r#"{"name":"test-pkg","version":"1.2.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta4", r#"{"name":"test-pkg","version":"2.0.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta5", r#"{"name":"test-pkg","version":"0.9.0"}"#, "author1", "pk1")?;
        
        // 设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.1.0", "meta2", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.2.0", "meta3", Some("beta"))?;
        meta_db.set_pkg_version("test-pkg", "2.0.0", "meta4", Some("alpha"))?;
        meta_db.set_pkg_version("test-pkg", "0.9.0", "meta5", Some("old"))?;
        
        // 测试获取最新版本（应该是2.0.0）
        let latest = meta_db.get_pkg_meta("test-pkg", None, None)?;
        assert!(latest.is_some());
        let (metaobjid, pkg_meta) = latest.unwrap();
        assert_eq!(metaobjid, "meta4");
        assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"2.0.0"}"#);
        
        // 测试获取特定版本
        let v1 = meta_db.get_pkg_meta("test-pkg", None, Some("1.1.0"))?;
        assert!(v1.is_some());
        let (metaobjid, pkg_meta) = v1.unwrap();
        assert_eq!(metaobjid, "meta2");
        assert_eq!(pkg_meta, r#"{"name":"test-pkg","version":"1.1.0"}"#);
        
        // 测试获取版本范围
        let versions = meta_db.get_versions_in_range("test-pkg", Some("1.0.0"), Some("1.2.0"))?;
        assert_eq!(versions.len(), 3);
        
        // 验证版本排序是否正确（应该是降序）
        assert_eq!(versions[0].0, "1.2.0");
        assert_eq!(versions[1].0, "1.1.0");
        assert_eq!(versions[2].0, "1.0.0");
        
        // 测试按标签获取
        let beta_version = meta_db.get_pkg_meta_by_tag("test-pkg", "beta")?;
        assert!(beta_version.is_some());
        let (metaobjid, _, _, _) = beta_version.unwrap();
        assert_eq!(metaobjid, "meta3");
        
        Ok(())
    }

    #[test]
    fn test_version_expr() -> PkgResult<()> {
        // 测试版本表达式解析
        let test_cases = vec![
            (">1.0.0", VersionOp::Gt, "1.0.0"),
            (">=1.0.0", VersionOp::Gte, "1.0.0"),
            ("<1.0.0", VersionOp::Lt, "1.0.0"),
            ("<=1.0.0", VersionOp::Lte, "1.0.0"),
            ("=1.0.0", VersionOp::Eq, "1.0.0"),
            ("1.0.0", VersionOp::Eq, "1.0.0"),
            ("^1.0.0", VersionOp::Caret, "1.0.0"),
            ("~1.0.0", VersionOp::Tilde, "1.0.0"),
        ];
        
        for (expr_str, expected_op, expected_version) in test_cases {
            let expr = VersionExpr::parse(expr_str)?;
            assert_eq!(expr.op, expected_op, "表达式 {} 的操作符应该是 {:?}", expr_str, expected_op);
            assert_eq!(expr.version, expected_version, "表达式 {} 的版本应该是 {}", expr_str, expected_version);
        }
        
        // 测试版本匹配
        let match_test_cases = vec![
            // 等于操作符
            ("=1.0.0", "1.0.0", true),
            ("=1.0.0", "1.0.1", false),
            ("1.0.0", "1.0.0", true),
            ("1.0.0", "1.0.1", false),
            
            // 大于操作符
            (">1.0.0", "1.0.1", true),
            (">1.0.0", "1.1.0", true),
            (">1.0.0", "2.0.0", true),
            (">1.0.0", "1.0.0", false),
            (">1.0.0", "0.9.0", false),
            
            // 小于操作符
            ("<1.0.0", "0.9.0", true),
            ("<1.0.0", "0.1.0", true),
            ("<1.0.0", "1.0.0", false),
            ("<1.0.0", "1.0.1", false),
            
            // 大于等于操作符
            (">=1.0.0", "1.0.0", true),
            (">=1.0.0", "1.0.1", true),
            (">=1.0.0", "0.9.0", false),
            
            // 小于等于操作符
            ("<=1.0.0", "1.0.0", true),
            ("<=1.0.0", "0.9.0", true),
            ("<=1.0.0", "1.0.1", false),
            
            // 插入符号操作符 (^)
            ("^1.0.0", "1.0.0", true),
            ("^1.0.0", "1.0.1", true),
            ("^1.0.0", "1.1.0", true),
            ("^1.0.0", "2.0.0", false),
            ("^0.1.0", "0.1.1", true),
            ("^0.1.0", "0.2.0", false),
            
            // 波浪号操作符 (~)
            ("~1.0.0", "1.0.0", true),
            ("~1.0.0", "1.0.1", true),
            ("~1.0.0", "1.1.0", false),
            ("~1.1.0", "1.1.1", true),
            ("~1.1.0", "1.2.0", false),
        ];
        
        for (expr_str, version, expected) in match_test_cases {
            let expr = VersionExpr::parse(expr_str)?;
            let result = expr.matches(version);
            assert_eq!(result, expected, "表达式 {} 对版本 {} 的匹配结果应该是 {}", expr_str, version, expected);
        }
        
        Ok(())
    }
    
    #[test]
    fn test_version_expr_db_operations() -> PkgResult<()> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_version_expr.db");
        
        let meta_db = MetaIndexDb::new(db_path)?;
        
        // 添加不同版本的包
        meta_db.add_pkg_meta("meta1", r#"{"name":"test-pkg","version":"1.0.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta2", r#"{"name":"test-pkg","version":"1.1.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta3", r#"{"name":"test-pkg","version":"1.2.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta4", r#"{"name":"test-pkg","version":"2.0.0"}"#, "author1", "pk1")?;
        meta_db.add_pkg_meta("meta5", r#"{"name":"test-pkg","version":"0.9.0"}"#, "author1", "pk1")?;
        
        // 设置包版本
        meta_db.set_pkg_version("test-pkg", "1.0.0", "meta1", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.1.0", "meta2", Some("stable"))?;
        meta_db.set_pkg_version("test-pkg", "1.2.0", "meta3", Some("beta"))?;
        meta_db.set_pkg_version("test-pkg", "2.0.0", "meta4", Some("alpha"))?;
        meta_db.set_pkg_version("test-pkg", "0.9.0", "meta5", Some("old"))?;
        
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
        let versions = meta_db.get_versions_by_expr("test-pkg", ">1.0.0")?;
        assert_eq!(versions.len(), 3, ">1.0.0 应该匹配3个版本");
        
        let versions = meta_db.get_versions_by_expr("test-pkg", "^1.0.0")?;
        assert_eq!(versions.len(), 3, "^1.0.0 应该匹配3个版本");
        
        let versions = meta_db.get_versions_by_expr("test-pkg", "~1.0.0")?;
        assert_eq!(versions.len(), 1, "~1.0.0 应该匹配1个版本");
        
        // 测试获取最大版本的包元数据
        let test_cases_max = vec![
            (">0.9.0", "meta4"),  // 应该获取2.0.0版本（最大的满足条件的版本）
            (">=1.0.0", "meta4"), // 应该获取2.0.0版本（最大的满足条件的版本）
            ("<2.0.0", "meta3"),  // 应该获取1.2.0版本（最大的满足条件的版本）
            ("<=1.2.0", "meta3"), // 应该获取1.2.0版本（最大的满足条件的版本）
            ("^1.0.0", "meta3"),  // 应该获取1.2.0版本（最大的满足条件的版本）
            ("~1.0.0", "meta1"),  // 应该获取1.0.0版本（最大的满足条件的版本）
        ];
        
        for (expr, expected_meta) in test_cases_max {
            let meta = meta_db.get_pkg_meta_by_expr_max("test-pkg", None, expr)?;
            assert!(meta.is_some(), "表达式 {} 应该匹配到版本", expr);
            let (metaobjid, _, _, _) = meta.unwrap();
            assert_eq!(metaobjid, expected_meta, "表达式 {} 应该匹配到 {}", expr, expected_meta);
        }
        
        Ok(())
    }
}