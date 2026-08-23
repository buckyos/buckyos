//! pikg（Personal AI Package）读取、索引、校验与打包。
//!
//! 真相源：doc/App 安装协议.md §4 与 §14.0 D1/D2。本模块只负责包格式与
//! 内容一致性（Document Syntax Validity / Package Integrity），不做 DID
//! 信任判断，不自动写 RepoService/NamedStore，不掺部署逻辑：
//! - 包内 App Document 永远是 candidate，Reader 不赋予任何发布状态；
//! - `load` 与 `import` 分离：读取只在事务内使用，落 NamedStore 由
//!   Prepare 的 materialize 策略显式决定；
//! - Reader 只应打开位于受控 staging root 下、以 pikg_digest 命名的
//!   immutable 文件（`stage_pikg_file`），防止校验后被替换（TOCTOU）。
//!
//! Packer（`PikgBuilder`）与 Verifier（`PikgReader`）共用同一套常量与
//! 命名规则，发布侧写完必须用同一个 Reader 自校验，禁止两套规则漂移。

use buckyos_api::{AppDoc, OBJ_TYPE_APP_DOC};
use ndn_lib::{build_named_object_by_json, ChunkId, ChunkType, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// 常量（v0.5 D1 冻结）
// ---------------------------------------------------------------------------

pub const PIKG_PACKAGE_META_SCHEMA: &str = "buckyos.pikg.package-meta.v1";
pub const PIKG_MIME_TYPE: &str = "application/vnd.buckyos.pikg+zip";
pub const PIKG_FILE_EXT: &str = "pikg";

pub const APPDOC_JWT_ENTRY: &str = "APPDOC.jwt";
pub const APPDOC_JSON_ENTRY: &str = "APPDOC.json";
pub const PACKAGE_META_ENTRY: &str = "PACKAGE_META.json";
pub const OBJECTS_PREFIX: &str = "objects/";
pub const CHUNKS_PREFIX: &str = "chunks/";
pub const ASSETS_PREFIX: &str = "assets/";

pub const PIKG_MAX_ENTRIES: usize = 4096;
pub const PIKG_MAX_APPDOC_BYTES: u64 = 1024 * 1024; // 1 MiB
pub const PIKG_MAX_METADATA_ENTRY_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB
pub const PIKG_MAX_METADATA_TOTAL_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

const ZIP_LOCAL_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const HASH_BUF_SIZE: usize = 256 * 1024;
const LEGACY_APPDOC_WT_ENTRY: &str = "APPDOC.wt";

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// pikg 层错误。`InvalidPackage` 对应协议错误码 `INVALID_PACKAGE`；
/// Installer 负责把它映射进结构化 `InstallError`。
#[derive(Debug, thiserror::Error)]
pub enum PikgError {
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    #[error("pikg io error: {0}")]
    Io(String),
    #[error("content missing in pikg: {0}")]
    ContentMissing(String),
}

pub type PikgResult<T> = std::result::Result<T, PikgError>;

fn invalid(msg: impl Into<String>) -> PikgError {
    PikgError::InvalidPackage(msg.into())
}

fn io_err(context: &str, err: impl std::fmt::Display) -> PikgError {
    PikgError::Io(format!("{context}: {err}"))
}

// ---------------------------------------------------------------------------
// PACKAGE_META.json 结构
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PikgContentIndexEntry {
    pub sub_pkg_name: String,
    pub path: String,
    pub format: String,
    pub size: u64,
    /// `sha256:<hex>`，针对 entry 解压后的最终字节（即 `.tar.gz` 文件本身）。
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PikgPackageMetaFile {
    #[serde(rename = "@schema")]
    pub schema: String,
    /// 必须等于包内 App Document 的 Object ID（`appdoc:<hex>`）。
    pub app_doc_id: String,
    /// key = Package Meta Object ID；value 按规范化规则重算必须得到同一 key。
    #[serde(default)]
    pub package_objects: BTreeMap<String, Value>,
    /// key = 内容 digest（`sha256:<hex>`），只允许指向包内真实 entry。
    #[serde(default)]
    pub content_index: BTreeMap<String, PikgContentIndexEntry>,
}

// ---------------------------------------------------------------------------
// 检查结果视图
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PikgEntryInfo {
    pub path: String,
    pub size: u64,
}

/// 打开并通过结构校验后的 pikg 视图。
/// 注意：`app_doc` 只是 candidate body；发布状态与 owner 信任由 Resolve 决定。
#[derive(Debug, Clone)]
pub struct PikgInspection {
    /// 整个 `.pikg` 文件字节的 sha256 hex。
    pub pikg_digest: String,
    pub app_doc: AppDoc,
    pub app_doc_object_id: ObjId,
    /// 包内是否带 `APPDOC.jwt`（签名封装）。签名验证属于 Resolve/Verify。
    pub has_signed_app_doc: bool,
    pub signed_app_doc_jwt: Option<String>,
    pub package_meta: PikgPackageMetaFile,
    pub entries: Vec<PikgEntryInfo>,
}

impl PikgInspection {
    pub fn content_entry(&self, content_id: &str) -> Option<&PikgContentIndexEntry> {
        self.package_meta.content_index.get(content_id).or_else(|| {
            self.package_meta.content_index.values().find(|entry| {
                let Some(desc) = self.app_doc.pkg_list.get(&entry.sub_pkg_name) else {
                    return false;
                };
                let Some(pkg_objid) = desc.pkg_objid.as_ref() else {
                    return false;
                };
                self.package_meta
                    .package_objects
                    .get(&pkg_objid.to_string())
                    .and_then(|value| value.get("content"))
                    .and_then(Value::as_str)
                    == Some(content_id)
            })
        })
    }

    /// 当前 pikg 内实际携带的内容 digest 集合。
    pub fn bundled_content_digests(&self) -> impl Iterator<Item = &str> {
        self.package_meta.content_index.keys().map(|s| s.as_str())
    }
}

// ---------------------------------------------------------------------------
// entry 名与结构校验
// ---------------------------------------------------------------------------

fn validate_entry_name(raw: &[u8]) -> PikgResult<String> {
    let name = std::str::from_utf8(raw)
        .map_err(|_| invalid("entry name is not valid utf-8"))?
        .to_string();
    if name.is_empty() {
        return Err(invalid("entry name is empty"));
    }
    if name.contains('\0') {
        return Err(invalid(format!("entry name contains NUL: {name:?}")));
    }
    if name.contains('\\') {
        return Err(invalid(format!(
            "entry name must use `/` separators only: {name:?}"
        )));
    }
    if name.starts_with('/') {
        return Err(invalid(format!("entry name is absolute: {name:?}")));
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(invalid(format!(
            "entry name looks like a drive path: {name:?}"
        )));
    }
    let is_dir = name.ends_with('/');
    let segments: Vec<&str> = name.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        if *segment == ".." || *segment == "." {
            return Err(invalid(format!(
                "entry name contains traversal segment: {name:?}"
            )));
        }
        let is_trailing_dir_marker = is_dir && index == segments.len() - 1;
        if segment.is_empty() && !is_trailing_dir_marker {
            return Err(invalid(format!(
                "entry name contains empty segment: {name:?}"
            )));
        }
    }
    Ok(name)
}

pub fn validate_sub_pkg_name(name: &str) -> PikgResult<()> {
    if name.is_empty() {
        return Err(invalid("sub_pkg_name is empty"));
    }
    if name == ".." || name == "." {
        return Err(invalid(format!(
            "sub_pkg_name is a traversal name: {name:?}"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(invalid(format!(
            "sub_pkg_name must match [A-Za-z0-9._-]+: {name:?}"
        )));
    }
    Ok(())
}

pub fn preferred_archive_name(sub_pkg_name: &str) -> String {
    format!("{sub_pkg_name}.tar.gz")
}

fn parse_sha256_digest(digest: &str) -> PikgResult<String> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid(format!("digest must be `sha256:<hex>`: {digest:?}")))?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid(format!("digest hex is invalid: {digest:?}")));
    }
    Ok(hex.to_ascii_lowercase())
}

/// 校验 PackageMeta.content（chunk id）与实测 (size, sha256) 一致。
/// 支持 sha256 与 mix256（varint(len) || sha256）两种 chunk 类型；
/// 其它类型显式判为不支持（而不是静默跳过）。
fn chunk_id_matches_content(
    chunk_id_str: &str,
    size: u64,
    sha256_bytes: &[u8],
) -> PikgResult<bool> {
    let chunk_id = ChunkId::new(chunk_id_str)
        .map_err(|err| invalid(format!("package meta content is not a chunk id: {err}")))?;
    match chunk_id.chunk_type {
        ChunkType::Sha256 => Ok(chunk_id.hash_result == sha256_bytes),
        ChunkType::Mix256 => {
            let expected = ChunkId::from_mix_hash_result(size, sha256_bytes, ChunkType::Mix256);
            Ok(chunk_id.hash_result == expected.hash_result)
        }
        other => Err(invalid(format!(
            "unsupported chunk type `{}` for pikg content cross-check",
            other.to_string()
        ))),
    }
}

fn is_metadata_entry(name: &str) -> bool {
    name == APPDOC_JWT_ENTRY
        || name == APPDOC_JSON_ENTRY
        || name == PACKAGE_META_ENTRY
        || (name.starts_with(OBJECTS_PREFIX) && name.ends_with(".json"))
}

fn metadata_entry_limit(name: &str) -> u64 {
    if name == APPDOC_JWT_ENTRY || name == APPDOC_JSON_ENTRY {
        PIKG_MAX_APPDOC_BYTES
    } else {
        PIKG_MAX_METADATA_ENTRY_BYTES
    }
}

// ---------------------------------------------------------------------------
// PikgReader
// ---------------------------------------------------------------------------

/// 只读 pikg 访问器。构造即完成结构校验（Inspect 级）；
/// payload hash 校验是显式操作（Verify 级），不在打开时全量执行。
pub struct PikgReader {
    path: PathBuf,
    inspection: PikgInspection,
    entry_index: HashMap<String, usize>,
}

impl PikgReader {
    /// 把来源文件原子化固定到 staging root（copy -> sha256 -> rename 为
    /// `{digest}.pikg`），返回 (pikg_digest_hex, staged_path)。
    /// 后续 Stage 不得再打开用户可替换的原路径。
    pub async fn stage_pikg_file(src: &Path, staging_root: &Path) -> PikgResult<(String, PathBuf)> {
        let src = src.to_path_buf();
        let staging_root = staging_root.to_path_buf();
        tokio::task::spawn_blocking(move || stage_pikg_file_blocking(&src, &staging_root))
            .await
            .map_err(|err| io_err("join stage_pikg_file", err))?
    }

    /// 打开 staging root 下的 immutable 文件并完成结构校验。
    /// `expected_digest` 提供时校验整个文件字节的 sha256。
    pub async fn open(path: &Path, expected_digest: Option<&str>) -> PikgResult<Self> {
        let path = path.to_path_buf();
        let expected = expected_digest.map(|value| value.to_string());
        tokio::task::spawn_blocking(move || Self::open_blocking(&path, expected.as_deref()))
            .await
            .map_err(|err| io_err("join pikg open", err))?
    }

    pub fn inspection(&self) -> &PikgInspection {
        &self.inspection
    }

    pub fn pikg_digest(&self) -> &str {
        &self.inspection.pikg_digest
    }

    pub fn staged_path(&self) -> &Path {
        &self.path
    }

    pub fn has_content(&self, digest: &str) -> bool {
        self.inspection.content_entry(digest).is_some()
    }

    /// Verify 级校验：流式解压 entry，重算 sha256 与字节数，
    /// 与 content_index 及（可用时）Package Meta 交叉核对。
    pub async fn verify_content(&self, digest: &str) -> PikgResult<()> {
        let entry = self
            .inspection
            .content_entry(digest)
            .cloned()
            .ok_or_else(|| PikgError::ContentMissing(digest.to_string()))?;
        let path = self.path.clone();
        let index = self.entry_index_of(&entry.path)?;
        let package_meta_content = self.package_meta_content_for(&entry);
        tokio::task::spawn_blocking(move || {
            verify_content_blocking(&path, index, &entry, package_meta_content.as_deref())
        })
        .await
        .map_err(|err| io_err("join verify_content", err))?
    }

    /// 校验 content_index 中的全部内容（完整包自校验/发布自检用）。
    pub async fn verify_all_contents(&self) -> PikgResult<()> {
        let digests: Vec<String> = self
            .inspection
            .package_meta
            .content_index
            .keys()
            .cloned()
            .collect();
        for digest in digests {
            self.verify_content(&digest).await?;
        }
        Ok(())
    }

    /// 事务内 Object Provider：按 ObjId 取包内结构化对象（canonical body 字符串）。
    /// 命中顺序：PACKAGE_META.json 的 package_objects -> objects/ 目录。
    /// 返回内容已经过 ObjId 重算验证；不自动写任何存储。
    pub async fn read_object(&self, obj_id: &ObjId) -> PikgResult<Option<String>> {
        let key = obj_id.to_string();
        if let Some(value) = self.inspection.package_meta.package_objects.get(&key) {
            let (computed, canonical) = build_named_object_by_json(&obj_id.obj_type, value);
            if computed != *obj_id {
                return Err(invalid(format!(
                    "package object `{key}` failed obj id recheck"
                )));
            }
            return Ok(Some(canonical));
        }

        let entry_name = format!("{OBJECTS_PREFIX}{key}.json");
        let Some(index) = self.entry_index.get(&entry_name).copied() else {
            return Ok(None);
        };
        let path = self.path.clone();
        let obj_id = obj_id.clone();
        tokio::task::spawn_blocking(move || {
            let mut archive = open_archive(&path)?;
            let mut entry = archive
                .by_index(index)
                .map_err(|err| io_err("open object entry", err))?;
            let bytes = read_entry_limited(&mut entry, PIKG_MAX_METADATA_ENTRY_BYTES, &entry_name)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
                invalid(format!("object entry `{entry_name}` is not json: {err}"))
            })?;
            let (computed, canonical) = build_named_object_by_json(&obj_id.obj_type, &value);
            if computed != obj_id {
                return Err(invalid(format!(
                    "object entry `{entry_name}` failed obj id recheck"
                )));
            }
            Ok(Some(canonical))
        })
        .await
        .map_err(|err| io_err("join read_object", err))?
    }

    /// 把某个内容 entry 验证后流式复制到 dest（部署 materialize 用）。
    /// 边复制边计算 hash，最后不一致则删除 dest 并报错。
    pub async fn copy_content_to_file(&self, digest: &str, dest: &Path) -> PikgResult<()> {
        let entry = self
            .inspection
            .content_entry(digest)
            .cloned()
            .ok_or_else(|| PikgError::ContentMissing(digest.to_string()))?;
        let index = self.entry_index_of(&entry.path)?;
        let path = self.path.clone();
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || copy_content_blocking(&path, index, &entry, &dest))
            .await
            .map_err(|err| io_err("join copy_content_to_file", err))?
    }

    pub async fn verify_package_archive(&self, digest: &str) -> PikgResult<()> {
        let entry = self
            .inspection
            .content_entry(digest)
            .cloned()
            .ok_or_else(|| PikgError::ContentMissing(digest.to_string()))?;
        let index = self.entry_index_of(&entry.path)?;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let mut archive = open_archive(&path)?;
            let zentry = archive
                .by_index(index)
                .map_err(|err| io_err("open package archive content", err))?;
            validate_package_archive(zentry)
        })
        .await
        .map_err(|err| io_err("join verify_package_archive", err))?
    }

    fn entry_index_of(&self, entry_path: &str) -> PikgResult<usize> {
        self.entry_index
            .get(entry_path)
            .copied()
            .ok_or_else(|| invalid(format!("content entry `{entry_path}` not found in pikg")))
    }

    /// 找到 content entry 对应的 Package Meta content（chunk id），供交叉校验。
    fn package_meta_content_for(&self, entry: &PikgContentIndexEntry) -> Option<String> {
        let desc = self.inspection.app_doc.pkg_list.get(&entry.sub_pkg_name)?;
        let pkg_objid = desc.pkg_objid.as_ref()?;
        let meta_value = self
            .inspection
            .package_meta
            .package_objects
            .get(&pkg_objid.to_string())?;
        meta_value
            .get("content")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn open_blocking(path: &Path, expected_digest: Option<&str>) -> PikgResult<Self> {
        // 1. magic 检查（扩展名只用于 UX，不作为格式判断）。
        let mut file = std::fs::File::open(path).map_err(|err| io_err("open pikg file", err))?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)
            .map_err(|err| invalid(format!("pikg file too small: {err}")))?;
        if magic != ZIP_LOCAL_MAGIC {
            return Err(invalid("pikg magic mismatch: not a zip container"));
        }

        // 2. 整文件 sha256（pikg_digest）。
        let pikg_digest = sha256_file_hex(path)?;
        if let Some(expected) = expected_digest {
            let expected = expected.trim().to_ascii_lowercase();
            let expected = expected.strip_prefix("sha256:").unwrap_or(&expected);
            if expected != pikg_digest {
                return Err(invalid(format!(
                    "pikg digest mismatch: expected {expected}, got {pikg_digest}"
                )));
            }
        }

        // 3. 自扫中央目录：entry 数上限 + 重复 entry（zip crate 会静默按名
        //    去重 last-wins，重复检测必须在库之外做）+ 全部原始名安全校验。
        let raw_names = scan_central_directory_names(path)?;
        {
            let mut seen: HashSet<String> = HashSet::new();
            for raw in &raw_names {
                let name = validate_entry_name(raw)?;
                if !seen.insert(name.clone()) {
                    return Err(invalid(format!("duplicate entry: {name:?}")));
                }
            }
        }

        let mut archive = open_archive(path)?;
        if archive.len() > PIKG_MAX_ENTRIES {
            return Err(invalid(format!(
                "too many entries: {} > {}",
                archive.len(),
                PIKG_MAX_ENTRIES
            )));
        }

        // 4. entry 名安全 + symlink + 目录/文件冲突 + metadata 总量。
        let mut entry_index: HashMap<String, usize> = HashMap::new();
        let mut entries: Vec<PikgEntryInfo> = Vec::new();
        let mut file_paths: HashSet<String> = HashSet::new();
        let mut dir_paths: HashSet<String> = HashSet::new();
        let mut metadata_total: u64 = 0;

        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|err| invalid(format!("read entry #{index} failed: {err}")))?;
            let name = validate_entry_name(entry.name_raw())?;

            if let Some(mode) = entry.unix_mode() {
                if mode & 0o170000 == 0o120000 {
                    return Err(invalid(format!("symlink entry is not allowed: {name:?}")));
                }
            }

            let is_dir = name.ends_with('/');
            let normalized = name.trim_end_matches('/').to_string();
            if is_dir {
                if !dir_paths.insert(normalized.clone()) {
                    return Err(invalid(format!("duplicate directory entry: {name:?}")));
                }
            } else {
                if !file_paths.insert(name.clone()) {
                    return Err(invalid(format!("duplicate entry: {name:?}")));
                }
                entry_index.insert(name.clone(), index);
                entries.push(PikgEntryInfo {
                    path: name.clone(),
                    size: entry.size(),
                });
                // 隐式目录前缀。
                let mut prefix = String::new();
                let segments: Vec<&str> = name.split('/').collect();
                for segment in &segments[..segments.len().saturating_sub(1)] {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(segment);
                    dir_paths.insert(prefix.clone());
                }
                if is_metadata_entry(&name) {
                    let limit = metadata_entry_limit(&name);
                    if entry.size() > limit {
                        return Err(invalid(format!(
                            "metadata entry `{name}` declared size {} exceeds limit {limit}",
                            entry.size()
                        )));
                    }
                    metadata_total = metadata_total.saturating_add(entry.size());
                }
            }
        }

        if entry_index.contains_key(LEGACY_APPDOC_WT_ENTRY) {
            return Err(invalid(
                "legacy APPDOC.wt entry is not supported; use APPDOC.jwt",
            ));
        }
        if metadata_total > PIKG_MAX_METADATA_TOTAL_BYTES {
            return Err(invalid(format!(
                "metadata total size {metadata_total} exceeds limit {PIKG_MAX_METADATA_TOTAL_BYTES}"
            )));
        }
        if let Some(conflict) = file_paths.intersection(&dir_paths).next() {
            return Err(invalid(format!(
                "path is both file and directory: {conflict:?}"
            )));
        }

        // 5. APPDOC：至少一个；两者都在时 canonical 一致（等价于 ObjId 相等）。
        let json_doc = match entry_index.get(APPDOC_JSON_ENTRY).copied() {
            Some(index) => {
                let mut entry = archive
                    .by_index(index)
                    .map_err(|err| io_err("open APPDOC.json", err))?;
                let bytes =
                    read_entry_limited(&mut entry, PIKG_MAX_APPDOC_BYTES, APPDOC_JSON_ENTRY)?;
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|err| invalid(format!("APPDOC.json is not valid json: {err}")))?;
                Some(value)
            }
            None => None,
        };
        let jwt_doc = match entry_index.get(APPDOC_JWT_ENTRY).copied() {
            Some(index) => {
                let mut entry = archive
                    .by_index(index)
                    .map_err(|err| io_err("open APPDOC.jwt", err))?;
                let bytes =
                    read_entry_limited(&mut entry, PIKG_MAX_APPDOC_BYTES, APPDOC_JWT_ENTRY)?;
                let jwt = String::from_utf8(bytes)
                    .map_err(|_| invalid("APPDOC.jwt is not utf-8"))?
                    .trim()
                    .to_string();
                let claims = name_lib::decode_jwt_claim_without_verify(jwt.as_str())
                    .map_err(|err| invalid(format!("APPDOC.jwt is not a decodable jwt: {err}")))?;
                Some((jwt, claims))
            }
            None => None,
        };

        let (app_doc_value, has_signed, signed_jwt) = match (&json_doc, &jwt_doc) {
            (None, None) => {
                return Err(invalid(
                    "pikg must contain APPDOC.jwt or APPDOC.json (none found)",
                ))
            }
            (Some(json_value), Some((jwt, claims))) => {
                let (json_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, json_value);
                let (jwt_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, claims);
                if json_id != jwt_id {
                    return Err(invalid(
                        "APPDOC.jwt and APPDOC.json express different canonical documents",
                    ));
                }
                // 默认优先采用签名版本的 claims（内容与 json 等价）。
                (claims.clone(), true, Some(jwt.clone()))
            }
            (Some(json_value), None) => (json_value.clone(), false, None),
            (None, Some((jwt, claims))) => (claims.clone(), true, Some(jwt.clone())),
        };

        let app_doc: AppDoc = serde_json::from_value(app_doc_value.clone())
            .map_err(|err| invalid(format!("app document schema invalid: {err}")))?;
        let (app_doc_object_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &app_doc_value);

        // 6. PACKAGE_META.json（必需，@schema 承载格式版本）。
        let package_meta_index = entry_index
            .get(PACKAGE_META_ENTRY)
            .copied()
            .ok_or_else(|| invalid("PACKAGE_META.json is required"))?;
        let package_meta: PikgPackageMetaFile = {
            let mut entry = archive
                .by_index(package_meta_index)
                .map_err(|err| io_err("open PACKAGE_META.json", err))?;
            let bytes = read_entry_limited(
                &mut entry,
                PIKG_MAX_METADATA_ENTRY_BYTES,
                PACKAGE_META_ENTRY,
            )?;
            serde_json::from_slice(&bytes)
                .map_err(|err| invalid(format!("PACKAGE_META.json invalid: {err}")))?
        };

        if package_meta.schema != PIKG_PACKAGE_META_SCHEMA {
            return Err(invalid(format!(
                "unsupported PACKAGE_META.json @schema `{}` (expect `{}`)",
                package_meta.schema, PIKG_PACKAGE_META_SCHEMA
            )));
        }
        if package_meta.app_doc_id != app_doc_object_id.to_string() {
            return Err(invalid(format!(
                "PACKAGE_META.json app_doc_id `{}` != actual app doc object id `{}`",
                package_meta.app_doc_id, app_doc_object_id
            )));
        }

        // 7. package_objects：key 与重算 ObjId 一致，且必须被 AppDoc 引用。
        let referenced_meta_ids: HashSet<String> = app_doc
            .pkg_list
            .iter()
            .into_iter()
            .filter_map(|(_, desc)| desc.pkg_objid.as_ref().map(|id| id.to_string()))
            .collect();
        for (key, value) in package_meta.package_objects.iter() {
            let key_obj_id = ObjId::new(key)
                .map_err(|err| invalid(format!("package_objects key `{key}` invalid: {err}")))?;
            let (computed, _) = build_named_object_by_json(&key_obj_id.obj_type, value);
            if computed.to_string() != *key {
                return Err(invalid(format!(
                    "package_objects `{key}` does not match recomputed obj id `{computed}`"
                )));
            }
            if !referenced_meta_ids.contains(key) {
                return Err(invalid(format!(
                    "package_objects `{key}` is not referenced by the app document"
                )));
            }
        }

        // 8. content_index：只指向真实 entry；命名、大小、digest、subpkg 关联一致。
        for (key, entry) in package_meta.content_index.iter() {
            parse_sha256_digest(key)?;
            if entry.digest != *key {
                return Err(invalid(format!(
                    "content_index key `{key}` != entry digest `{}`",
                    entry.digest
                )));
            }
            validate_sub_pkg_name(&entry.sub_pkg_name)?;
            if entry.format == "tar.gz" {
                let expected = preferred_archive_name(&entry.sub_pkg_name);
                if entry.path != expected {
                    return Err(invalid(format!(
                        "content path `{}` must be `{expected}` for tar.gz sub package `{}`",
                        entry.path, entry.sub_pkg_name
                    )));
                }
            }
            let zip_index = entry_index.get(&entry.path).copied().ok_or_else(|| {
                invalid(format!(
                    "content_index path `{}` does not exist in pikg",
                    entry.path
                ))
            })?;
            let declared = {
                let zentry = archive
                    .by_index(zip_index)
                    .map_err(|err| io_err("open content entry", err))?;
                zentry.size()
            };
            if declared != entry.size {
                return Err(invalid(format!(
                    "content `{}` size mismatch: zip declares {declared}, index says {}",
                    entry.path, entry.size
                )));
            }
            // 关联 subpackage 必须在 AppDoc 中存在。
            let Some(desc) = app_doc.pkg_list.get(&entry.sub_pkg_name) else {
                return Err(invalid(format!(
                    "content sub package `{}` is not declared by the app document",
                    entry.sub_pkg_name
                )));
            };
            // 有 Package Meta 时核对 size（hash 交叉在 Verify 做）。
            if let Some(pkg_objid) = desc.pkg_objid.as_ref() {
                if let Some(meta_value) = package_meta.package_objects.get(&pkg_objid.to_string()) {
                    if let Some(meta_size) = meta_value.get("size").and_then(|v| v.as_u64()) {
                        if meta_size != 0 && meta_size != entry.size {
                            return Err(invalid(format!(
                                "content `{}` size {} != package meta size {meta_size}",
                                entry.path, entry.size
                            )));
                        }
                    }
                }
            }
        }

        let inspection = PikgInspection {
            pikg_digest,
            app_doc,
            app_doc_object_id,
            has_signed_app_doc: has_signed,
            signed_app_doc_jwt: signed_jwt,
            package_meta,
            entries,
        };

        Ok(Self {
            path: path.to_path_buf(),
            inspection,
            entry_index,
        })
    }
}

// ---------------------------------------------------------------------------
// blocking 工具
// ---------------------------------------------------------------------------

fn open_archive(path: &Path) -> PikgResult<zip::ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).map_err(|err| io_err("open pikg file", err))?;
    zip::ZipArchive::new(file).map_err(|err| invalid(format!("not a readable zip: {err}")))
}

const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
const EOCD64_LOCATOR_SIG: [u8; 4] = [0x50, 0x4B, 0x06, 0x07];
const EOCD64_SIG: [u8; 4] = [0x50, 0x4B, 0x06, 0x06];
const CENTRAL_DIR_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
const MAX_CENTRAL_DIR_BYTES: u64 = 64 * 1024 * 1024;

fn read_u16(bytes: &[u8], offset: usize) -> u64 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as u64
}

fn read_u32(bytes: &[u8], offset: usize) -> u64 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]) as u64
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// 自行扫描 ZIP 中央目录，返回按记录顺序的原始 entry 名。
/// zip crate 解析时会按名字去重（last-wins），从它的视角看不到重复 entry；
/// 而"重复 entry 必须拒绝"是 D1 的安全规则（防同名后写覆盖先校验内容），
/// 所以重复检测必须在这里做。支持 ZIP64。
fn scan_central_directory_names(path: &Path) -> PikgResult<Vec<Vec<u8>>> {
    use std::io::{Seek, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|err| io_err("open for scan", err))?;
    let file_len = file
        .metadata()
        .map_err(|err| io_err("stat for scan", err))?
        .len();

    // 定位 EOCD（末尾最多 22 + 65535 字节内）。
    let tail_len = file_len.min(22 + 65535);
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|err| io_err("seek eocd", err))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)
        .map_err(|err| io_err("read eocd", err))?;
    let eocd_pos_in_tail = tail
        .windows(4)
        .rposition(|window| window == EOCD_SIG)
        .ok_or_else(|| invalid("zip end-of-central-directory not found"))?;
    let eocd = &tail[eocd_pos_in_tail..];
    if eocd.len() < 22 {
        return Err(invalid("truncated end-of-central-directory record"));
    }
    let mut total_entries = read_u16(eocd, 10);
    let mut central_size = read_u32(eocd, 12);
    let mut central_offset = read_u32(eocd, 16);

    // ZIP64：EOCD 字段打满时读 ZIP64 EOCD。
    if total_entries == 0xFFFF || central_size == 0xFFFF_FFFF || central_offset == 0xFFFF_FFFF {
        let eocd_abs = file_len - tail_len + eocd_pos_in_tail as u64;
        if eocd_abs < 20 {
            return Err(invalid("zip64 locator missing"));
        }
        file.seek(SeekFrom::Start(eocd_abs - 20))
            .map_err(|err| io_err("seek zip64 locator", err))?;
        let mut locator = [0u8; 20];
        file.read_exact(&mut locator)
            .map_err(|err| io_err("read zip64 locator", err))?;
        if locator[0..4] != EOCD64_LOCATOR_SIG {
            return Err(invalid("zip64 locator signature mismatch"));
        }
        let eocd64_offset = read_u64(&locator, 8);
        file.seek(SeekFrom::Start(eocd64_offset))
            .map_err(|err| io_err("seek zip64 eocd", err))?;
        let mut eocd64 = [0u8; 56];
        file.read_exact(&mut eocd64)
            .map_err(|err| io_err("read zip64 eocd", err))?;
        if eocd64[0..4] != EOCD64_SIG {
            return Err(invalid("zip64 eocd signature mismatch"));
        }
        total_entries = read_u64(&eocd64, 32);
        central_size = read_u64(&eocd64, 40);
        central_offset = read_u64(&eocd64, 48);
    }

    if central_size > MAX_CENTRAL_DIR_BYTES {
        return Err(invalid(format!(
            "central directory too large: {central_size} bytes"
        )));
    }
    if total_entries as usize > PIKG_MAX_ENTRIES {
        return Err(invalid(format!(
            "too many entries: {total_entries} > {PIKG_MAX_ENTRIES}"
        )));
    }

    file.seek(SeekFrom::Start(central_offset))
        .map_err(|err| io_err("seek central directory", err))?;
    let mut central = vec![0u8; central_size as usize];
    file.read_exact(&mut central)
        .map_err(|err| io_err("read central directory", err))?;

    let mut names = Vec::with_capacity(total_entries as usize);
    let mut pos = 0usize;
    while (names.len() as u64) < total_entries {
        if pos + 46 > central.len() {
            return Err(invalid("truncated central directory record"));
        }
        if central[pos..pos + 4] != CENTRAL_DIR_SIG {
            return Err(invalid("central directory record signature mismatch"));
        }
        let name_len = read_u16(&central, pos + 28) as usize;
        let extra_len = read_u16(&central, pos + 30) as usize;
        let comment_len = read_u16(&central, pos + 32) as usize;
        let name_start = pos + 46;
        let name_end = name_start + name_len;
        if name_end > central.len() {
            return Err(invalid("central directory name out of bounds"));
        }
        names.push(central[name_start..name_end].to_vec());
        pos = name_end + extra_len + comment_len;
    }
    Ok(names)
}

fn sha256_file_hex(path: &Path) -> PikgResult<String> {
    let mut file = std::fs::File::open(path).map_err(|err| io_err("open for digest", err))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| io_err("read for digest", err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 读取 metadata entry：declared size 与实际读取都不得超过 limit。
fn read_entry_limited<R: Read>(entry: &mut R, limit: u64, label: &str) -> PikgResult<Vec<u8>> {
    let mut buf = Vec::new();
    let read = entry
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|err| invalid(format!("read `{label}` failed: {err}")))?;
    if read as u64 > limit {
        return Err(invalid(format!(
            "`{label}` exceeds size limit {limit} bytes"
        )));
    }
    Ok(buf)
}

fn stage_pikg_file_blocking(src: &Path, staging_root: &Path) -> PikgResult<(String, PathBuf)> {
    std::fs::create_dir_all(staging_root).map_err(|err| io_err("create staging root", err))?;
    let staging_root_canonical = staging_root
        .canonicalize()
        .map_err(|err| io_err("canonicalize staging root", err))?;

    let mut src_file = std::fs::File::open(src).map_err(|err| io_err("open source pikg", err))?;
    let tmp_path = staging_root_canonical.join(format!(
        ".staging-{}-{}.tmp",
        std::process::id(),
        buckyos_kit::buckyos_get_unix_timestamp()
    ));
    let mut tmp_file =
        std::fs::File::create(&tmp_path).map_err(|err| io_err("create staging tmp", err))?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let read = src_file
            .read(&mut buf)
            .map_err(|err| io_err("read source pikg", err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        tmp_file
            .write_all(&buf[..read])
            .map_err(|err| io_err("write staging tmp", err))?;
    }
    tmp_file
        .sync_all()
        .map_err(|err| io_err("sync staging tmp", err))?;
    drop(tmp_file);

    let digest = hex::encode(hasher.finalize());
    let final_path = staging_root_canonical.join(format!("{digest}.{PIKG_FILE_EXT}"));
    if final_path.exists() {
        let _ = std::fs::remove_file(&tmp_path);
    } else {
        std::fs::rename(&tmp_path, &final_path).map_err(|err| io_err("rename staging", err))?;
    }

    // handle 解析结果必须仍位于 staging root 内（D5）。
    let canonical = final_path
        .canonicalize()
        .map_err(|err| io_err("canonicalize staged file", err))?;
    if !canonical.starts_with(&staging_root_canonical) {
        return Err(invalid("staged file escaped staging root"));
    }
    Ok((digest, final_path))
}

fn verify_content_blocking(
    path: &Path,
    index: usize,
    entry: &PikgContentIndexEntry,
    package_meta_content: Option<&str>,
) -> PikgResult<()> {
    let mut archive = open_archive(path)?;
    let mut zentry = archive
        .by_index(index)
        .map_err(|err| io_err("open content entry", err))?;

    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let read = zentry
            .read(&mut buf)
            .map_err(|err| invalid(format!("read content `{}` failed: {err}", entry.path)))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        total += read as u64;
        if total > entry.size {
            return Err(invalid(format!(
                "content `{}` larger than declared size {}",
                entry.path, entry.size
            )));
        }
    }
    if total != entry.size {
        return Err(invalid(format!(
            "content `{}` size mismatch: declared {}, actual {total}",
            entry.path, entry.size
        )));
    }
    let sha256_bytes = hasher.finalize().to_vec();
    let actual_hex = hex::encode(&sha256_bytes);
    let expected_hex = parse_sha256_digest(&entry.digest)?;
    if actual_hex != expected_hex {
        return Err(invalid(format!(
            "content `{}` digest mismatch: declared sha256:{expected_hex}, actual sha256:{actual_hex}",
            entry.path
        )));
    }

    // Package Meta 交叉校验（sha256 / mix256 chunk id）。
    if let Some(chunk_id_str) = package_meta_content {
        if !chunk_id_matches_content(chunk_id_str, entry.size, &sha256_bytes)? {
            return Err(invalid(format!(
                "content `{}` does not match package meta chunk id `{chunk_id_str}`",
                entry.path
            )));
        }
    }
    Ok(())
}

fn copy_content_blocking(
    path: &Path,
    index: usize,
    entry: &PikgContentIndexEntry,
    dest: &Path,
) -> PikgResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|err| io_err("create dest dir", err))?;
    }
    let mut archive = open_archive(path)?;
    let mut zentry = archive
        .by_index(index)
        .map_err(|err| io_err("open content entry", err))?;
    let mut out = std::fs::File::create(dest).map_err(|err| io_err("create dest file", err))?;

    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    let result: PikgResult<()> = (|| {
        loop {
            let read = zentry
                .read(&mut buf)
                .map_err(|err| invalid(format!("read content `{}` failed: {err}", entry.path)))?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
            out.write_all(&buf[..read])
                .map_err(|err| io_err("write dest file", err))?;
            total += read as u64;
            if total > entry.size {
                return Err(invalid(format!(
                    "content `{}` larger than declared size {}",
                    entry.path, entry.size
                )));
            }
        }
        if total != entry.size {
            return Err(invalid(format!(
                "content `{}` size mismatch on copy: declared {}, actual {total}",
                entry.path, entry.size
            )));
        }
        let actual_hex = hex::encode(hasher.finalize_reset());
        let expected_hex = parse_sha256_digest(&entry.digest)?;
        if actual_hex != expected_hex {
            return Err(invalid(format!(
                "content `{}` digest mismatch on copy",
                entry.path
            )));
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn validate_package_archive(reader: impl Read) -> PikgResult<()> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|error| invalid(format!("invalid package archive: {error}")))?
    {
        let entry = entry.map_err(|error| invalid(format!("invalid package archive: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| invalid(format!("invalid package archive path: {error}")))?;
        let path_text = path.to_string_lossy();
        let windows_absolute = path_text.as_bytes().get(1) == Some(&b':');
        if path.is_absolute()
            || path_text.starts_with(['/', '\\'])
            || path_text.contains('\\')
            || windows_absolute
            || path.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(invalid(format!(
                "package archive entry `{path_text}` escapes its install root"
            )));
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(invalid(format!(
                "package archive entry `{path_text}` uses a link"
            )));
        }
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(invalid(format!(
                "package archive entry `{path_text}` has an unsupported type"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PikgBuilder（发布与测试共用的 packer）
// ---------------------------------------------------------------------------

struct BuilderPayload {
    sub_pkg_name: String,
    file_path: PathBuf,
}

pub struct PikgBuildOutput {
    pub pikg_digest: String,
    pub app_doc_object_id: ObjId,
    pub package_meta: PikgPackageMetaFile,
}

/// pikg 打包器。生成的包必须能通过同一模块的 `PikgReader` 校验；
/// 发布侧写完文件后应立即用 Reader 自校验。
pub struct PikgBuilder {
    app_doc_value: Option<Value>,
    app_doc_jwt: Option<String>,
    package_metas: BTreeMap<String, Value>,
    payloads: Vec<BuilderPayload>,
    extra_objects: Vec<(ObjId, Value)>,
}

impl PikgBuilder {
    pub fn new() -> Self {
        Self {
            app_doc_value: None,
            app_doc_jwt: None,
            package_metas: BTreeMap::new(),
            payloads: Vec::new(),
            extra_objects: Vec::new(),
        }
    }

    pub fn app_doc(mut self, app_doc: &AppDoc) -> PikgResult<Self> {
        let value = serde_json::to_value(app_doc)
            .map_err(|err| invalid(format!("serialize app doc failed: {err}")))?;
        self.app_doc_value = Some(value);
        Ok(self)
    }

    /// 直接提供签名封装的 App Document（JWT）。claims 必须与 app_doc 一致
    /// （由 write 后的 Reader 自校验兜底）。
    pub fn app_doc_jwt(mut self, jwt: impl Into<String>) -> Self {
        self.app_doc_jwt = Some(jwt.into());
        self
    }

    /// 注册一个 Package Meta 对象（obj id 由 canonical 重算得出）。
    pub fn add_package_meta_value(mut self, value: Value) -> PikgResult<(Self, ObjId)> {
        let (obj_id, _) = build_named_object_by_json(ndn_lib::OBJ_TYPE_PKG, &value);
        self.package_metas.insert(obj_id.to_string(), value);
        Ok((self, obj_id))
    }

    /// 添加一个 subpackage 的实体归档（应为最终 `.tar.gz` 文件）。
    /// 包内 entry 名固定为 `{sub_pkg_name}.tar.gz`。
    pub fn add_payload_file(
        mut self,
        sub_pkg_name: impl Into<String>,
        file_path: impl Into<PathBuf>,
    ) -> PikgResult<Self> {
        let sub_pkg_name = sub_pkg_name.into();
        validate_sub_pkg_name(&sub_pkg_name)?;
        self.payloads.push(BuilderPayload {
            sub_pkg_name,
            file_path: file_path.into(),
        });
        Ok(self)
    }

    /// 附带其它结构化对象（写入 objects/<objid>.json）。
    pub fn add_object(mut self, obj_id: ObjId, value: Value) -> Self {
        self.extra_objects.push((obj_id, value));
        self
    }

    pub async fn write_to(self, dest: &Path) -> PikgResult<PikgBuildOutput> {
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || self.write_blocking(&dest))
            .await
            .map_err(|err| io_err("join pikg write", err))?
    }

    fn write_blocking(self, dest: &Path) -> PikgResult<PikgBuildOutput> {
        use zip::write::SimpleFileOptions;

        let app_doc_value = self
            .app_doc_value
            .clone()
            .or_else(|| {
                self.app_doc_jwt
                    .as_ref()
                    .and_then(|jwt| name_lib::decode_jwt_claim_without_verify(jwt.as_str()).ok())
            })
            .ok_or_else(|| invalid("PikgBuilder requires an app document"))?;
        let (app_doc_object_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &app_doc_value);

        // 计算 payload digest / size，构造 content_index。
        let mut content_index: BTreeMap<String, PikgContentIndexEntry> = BTreeMap::new();
        for payload in &self.payloads {
            let size = std::fs::metadata(&payload.file_path)
                .map_err(|err| io_err("stat payload", err))?
                .len();
            let digest_hex = sha256_file_hex(&payload.file_path)?;
            let digest = format!("sha256:{digest_hex}");
            content_index.insert(
                digest.clone(),
                PikgContentIndexEntry {
                    sub_pkg_name: payload.sub_pkg_name.clone(),
                    path: preferred_archive_name(&payload.sub_pkg_name),
                    format: "tar.gz".to_string(),
                    size,
                    digest,
                },
            );
        }

        let package_meta = PikgPackageMetaFile {
            schema: PIKG_PACKAGE_META_SCHEMA.to_string(),
            app_doc_id: app_doc_object_id.to_string(),
            package_objects: self.package_metas.clone(),
            content_index,
        };

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| io_err("create dest dir", err))?;
        }
        let file = std::fs::File::create(dest).map_err(|err| io_err("create pikg file", err))?;
        let mut writer = zip::ZipWriter::new(file);
        let meta_options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        // payload 已是压缩产物，用 stored 避免二次压缩（D1 推荐）。
        let payload_options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);

        if let Some(jwt) = self.app_doc_jwt.as_ref() {
            writer
                .start_file(APPDOC_JWT_ENTRY, meta_options)
                .map_err(|err| io_err("start APPDOC.jwt", err))?;
            writer
                .write_all(jwt.as_bytes())
                .map_err(|err| io_err("write APPDOC.jwt", err))?;
        }
        if self.app_doc_value.is_some() {
            let body = serde_json::to_string_pretty(&app_doc_value)
                .map_err(|err| invalid(format!("serialize app doc: {err}")))?;
            writer
                .start_file(APPDOC_JSON_ENTRY, meta_options)
                .map_err(|err| io_err("start APPDOC.json", err))?;
            writer
                .write_all(body.as_bytes())
                .map_err(|err| io_err("write APPDOC.json", err))?;
        }

        let meta_body = serde_json::to_string_pretty(&package_meta)
            .map_err(|err| invalid(format!("serialize PACKAGE_META.json: {err}")))?;
        writer
            .start_file(PACKAGE_META_ENTRY, meta_options)
            .map_err(|err| io_err("start PACKAGE_META.json", err))?;
        writer
            .write_all(meta_body.as_bytes())
            .map_err(|err| io_err("write PACKAGE_META.json", err))?;

        for (obj_id, value) in &self.extra_objects {
            let (recomputed, canonical) = build_named_object_by_json(&obj_id.obj_type, value);
            if recomputed != *obj_id {
                return Err(invalid(format!(
                    "extra object `{obj_id}` does not match recomputed id `{recomputed}`"
                )));
            }
            writer
                .start_file(format!("{OBJECTS_PREFIX}{obj_id}.json"), meta_options)
                .map_err(|err| io_err("start object entry", err))?;
            writer
                .write_all(canonical.as_bytes())
                .map_err(|err| io_err("write object entry", err))?;
        }

        for payload in &self.payloads {
            let entry_name = preferred_archive_name(&payload.sub_pkg_name);
            writer
                .start_file(entry_name.as_str(), payload_options)
                .map_err(|err| io_err("start payload entry", err))?;
            let mut src = std::fs::File::open(&payload.file_path)
                .map_err(|err| io_err("open payload", err))?;
            std::io::copy(&mut src, &mut writer).map_err(|err| io_err("write payload", err))?;
        }

        writer
            .finish()
            .map_err(|err| io_err("finalize pikg", err))?;

        let pikg_digest = sha256_file_hex(dest)?;
        Ok(PikgBuildOutput {
            pikg_digest,
            app_doc_object_id,
            package_meta,
        })
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AppType, SubPkgDesc};
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use name_lib::DID;
    use package_lib::PackageMeta;
    use std::io::Cursor;
    use uuid::Uuid;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pikg-test-{prefix}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn make_tar_gz(dir: &Path, name: &str, inner: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).expect("create tar.gz");
        let encoder = GzEncoder::new(file, Compression::default());
        let mut tar = tar::Builder::new(encoder);
        for (entry_name, bytes) in inner {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, entry_name, Cursor::new(bytes.to_vec()))
                .expect("append tar entry");
        }
        tar.finish().expect("finish tar");
        path
    }

    /// 构造一个结构合法的 (AppDoc, PackageMeta, payload)。
    struct Fixture {
        dir: PathBuf,
        app_doc: AppDoc,
        payload_path: PathBuf,
        meta_value: Value,
    }

    fn build_fixture() -> Fixture {
        let dir = temp_dir("fixture");
        let payload_path = make_tar_gz(&dir, "web-src.tar.gz", &[("index.html", b"hello")]);
        let size = std::fs::metadata(&payload_path).unwrap().len();
        let digest_hex = sha256_file_hex(&payload_path).unwrap();

        let owner = DID::from_str("did:bns:tester").unwrap();
        let mut meta = PackageMeta::new(
            "all.web.demo-web.tester.bns.did",
            "0.1.0",
            "tester",
            &owner,
            None,
        );
        meta.size = size;
        // content = sha256 chunk id，与 payload 一致，供 Verify 交叉校验。
        meta.content = format!("sha256:{digest_hex}");
        let meta_value = serde_json::to_value(&meta).unwrap();
        let (meta_obj_id, _) = build_named_object_by_json(ndn_lib::OBJ_TYPE_PKG, &meta_value);

        let mut web_desc = SubPkgDesc::new("all.web.demo-web.tester.bns.did#0.1.0");
        web_desc.pkg_objid = Some(meta_obj_id);
        let app_doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(web_desc)
            .build()
            .unwrap();

        Fixture {
            dir,
            app_doc,
            payload_path,
            meta_value,
        }
    }

    async fn build_valid_pikg(fixture: &Fixture) -> PathBuf {
        let dest = fixture.dir.join("demo.pikg");
        let builder = PikgBuilder::new().app_doc(&fixture.app_doc).unwrap();
        let (builder, _meta_id) = builder
            .add_package_meta_value(fixture.meta_value.clone())
            .unwrap();
        let builder = builder
            .add_payload_file("web", fixture.payload_path.clone())
            .unwrap();
        builder.write_to(&dest).await.unwrap();
        dest
    }

    /// 低层 zip 重写：对合法包做定点破坏。
    /// mutate: (entry_name, bytes) -> None 表示删除该 entry，Some 表示替换内容。
    /// extra: 追加 entry（可与已有重名，用于重复 entry 用例）。
    fn rewrite_zip(
        src: &Path,
        dest: &Path,
        mut mutate: impl FnMut(&str, Vec<u8>) -> Option<Vec<u8>>,
        extra: Vec<(String, Vec<u8>)>,
    ) {
        use zip::write::SimpleFileOptions;
        let mut archive = open_archive(src).unwrap();
        let out = std::fs::File::create(dest).unwrap();
        let mut writer = zip::ZipWriter::new(out);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let name = String::from_utf8(entry.name_raw().to_vec()).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            match mutate(&name, bytes) {
                Some(new_bytes) => {
                    writer.start_file(name.as_str(), options).unwrap();
                    writer.write_all(&new_bytes).unwrap();
                }
                None => continue,
            }
        }
        for (name, bytes) in extra {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn expect_invalid(result: PikgResult<PikgReader>, needle: &str) {
        match result {
            Ok(_) => panic!("expected InvalidPackage containing `{needle}`"),
            Err(PikgError::InvalidPackage(msg)) => {
                assert!(
                    msg.contains(needle),
                    "expected error containing `{needle}`, got: {msg}"
                );
            }
            Err(other) => panic!("expected InvalidPackage, got: {other}"),
        }
    }

    #[tokio::test]
    async fn valid_partial_pikg_roundtrip() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        let reader = PikgReader::open(&pikg_path, None).await.unwrap();
        let inspection = reader.inspection();
        assert_eq!(
            inspection.app_doc.did.to_string(),
            "did:bns:demo-web.tester"
        );
        assert_eq!(inspection.app_doc_object_id.obj_type, OBJ_TYPE_APP_DOC);
        assert!(!inspection.has_signed_app_doc);
        assert_eq!(inspection.package_meta.content_index.len(), 1);

        // Verify 级全量校验通过（含 PackageMeta chunk 交叉核对）。
        reader.verify_all_contents().await.unwrap();

        // Object provider：按 ObjId 取回 Package Meta。
        let meta_id = inspection
            .app_doc
            .pkg_list
            .web
            .as_ref()
            .unwrap()
            .pkg_objid
            .clone()
            .unwrap();
        let body = reader.read_object(&meta_id).await.unwrap().unwrap();
        let parsed: PackageMeta = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.version, "0.1.0");
        reader.verify_content(&parsed.content).await.unwrap();
        reader
            .verify_package_archive(&parsed.content)
            .await
            .unwrap();

        // 未知对象返回 None（不报错）。
        let missing = ObjId::new_by_raw("pkg".to_string(), vec![9u8; 32]);
        assert!(reader.read_object(&missing).await.unwrap().is_none());

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn signed_appdoc_uses_jwt_entry_name() {
        let fixture = build_fixture();
        let app_doc_value = serde_json::to_value(&fixture.app_doc).unwrap();
        let header = base64_url_encode(br#"{"alg":"EdDSA"}"#);
        let payload = base64_url_encode(serde_json::to_string(&app_doc_value).unwrap().as_bytes());
        let fake_jwt = format!("{header}.{payload}.c2ln");
        let pikg_path = fixture.dir.join("signed.pikg");

        let builder = PikgBuilder::new()
            .app_doc(&fixture.app_doc)
            .unwrap()
            .app_doc_jwt(fake_jwt.clone());
        let (builder, _meta_id) = builder
            .add_package_meta_value(fixture.meta_value.clone())
            .unwrap();
        builder
            .add_payload_file("web", fixture.payload_path.clone())
            .unwrap()
            .write_to(&pikg_path)
            .await
            .unwrap();

        let mut archive = open_archive(&pikg_path).unwrap();
        let entry_names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect::<Vec<_>>();
        assert!(entry_names.iter().any(|name| name == APPDOC_JWT_ENTRY));
        assert!(!entry_names
            .iter()
            .any(|name| name == LEGACY_APPDOC_WT_ENTRY));

        let reader = PikgReader::open(&pikg_path, None).await.unwrap();
        assert!(reader.inspection().has_signed_app_doc);
        assert_eq!(
            reader.inspection().signed_app_doc_jwt.as_deref(),
            Some(fake_jwt.as_str())
        );

        // 旧 entry 名不再兼容，也不能在 APPDOC.json 存在时静默降级。
        let legacy_path = fixture.dir.join("legacy-wt.pikg");
        rewrite_zip(
            &pikg_path,
            &legacy_path,
            |name, bytes| {
                if name == APPDOC_JWT_ENTRY {
                    None
                } else {
                    Some(bytes)
                }
            },
            vec![(LEGACY_APPDOC_WT_ENTRY.to_string(), fake_jwt.into_bytes())],
        );
        expect_invalid(
            PikgReader::open(&legacy_path, None).await,
            "legacy APPDOC.wt entry is not supported",
        );

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn staging_fixes_digest_and_detects_tamper() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        let staging_root = fixture.dir.join("staging");
        let (digest, staged) = PikgReader::stage_pikg_file(&pikg_path, &staging_root)
            .await
            .unwrap();
        assert!(staged
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&digest));

        // 篡改原文件不影响 staged 副本（TOCTOU 防护）。
        std::fs::write(&pikg_path, b"tampered").unwrap();
        let reader = PikgReader::open(&staged, Some(&digest)).await.unwrap();
        reader.verify_all_contents().await.unwrap();

        // 篡改 staged 文件后按 digest 打开必须失败。
        let mut bytes = std::fs::read(&staged).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&staged, &bytes).unwrap();
        let result = PikgReader::open(&staged, Some(&digest)).await;
        assert!(matches!(result, Err(PikgError::InvalidPackage(_))));

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn rejects_non_zip_and_missing_appdoc() {
        let dir = temp_dir("bad-magic");
        let not_zip = dir.join("fake.pikg");
        std::fs::write(&not_zip, b"this is not a zip file").unwrap();
        expect_invalid(PikgReader::open(&not_zip, None).await, "magic");

        // 有效 zip 但没有 APPDOC。
        let empty_zip = dir.join("empty.pikg");
        {
            use zip::write::SimpleFileOptions;
            let file = std::fs::File::create(&empty_zip).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("assets/readme.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"hi").unwrap();
            writer.finish().unwrap();
        }
        expect_invalid(PikgReader::open(&empty_zip, None).await, "APPDOC");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_path_traversal_and_symlink_entries() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        // `..` 穿越。
        let bad = fixture.dir.join("traversal.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |_, bytes| Some(bytes),
            vec![("../escape.txt".to_string(), b"boom".to_vec())],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "traversal");

        // 绝对路径。
        let bad = fixture.dir.join("absolute.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |_, bytes| Some(bytes),
            vec![("/etc/passwd".to_string(), b"boom".to_vec())],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "absolute");

        // 反斜杠。
        let bad = fixture.dir.join("backslash.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |_, bytes| Some(bytes),
            vec![("a\\b.txt".to_string(), b"boom".to_vec())],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "separators");

        // symlink entry。
        let bad = fixture.dir.join("symlink.pikg");
        {
            use zip::write::SimpleFileOptions;
            let mut archive = open_archive(&pikg_path).unwrap();
            let out = std::fs::File::create(&bad).unwrap();
            let mut writer = zip::ZipWriter::new(out);
            let options = SimpleFileOptions::default();
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                let name = String::from_utf8(entry.name_raw().to_vec()).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                writer.start_file(name.as_str(), options).unwrap();
                writer.write_all(&bytes).unwrap();
            }
            writer
                .add_symlink("evil-link", "/etc/passwd", options)
                .unwrap();
            writer.finish().unwrap();
        }
        expect_invalid(PikgReader::open(&bad, None).await, "symlink");

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn rejects_duplicate_entries() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        // zip writer 自身拒绝重名，改为字节级补丁：先加一个同长度的占位
        // entry `APPDOC.jsoX`，再把它的名字（local header + central dir 两处）
        // 改写成 `APPDOC.json`，构造真正的重复 entry。
        let bad = fixture.dir.join("dup.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |_, bytes| Some(bytes),
            vec![("APPDOC.jsoX".to_string(), b"{}".to_vec())],
        );
        let mut bytes = std::fs::read(&bad).unwrap();
        let needle = b"APPDOC.jsoX";
        let replacement = b"APPDOC.json";
        let mut start = 0usize;
        let mut patched = 0;
        while let Some(pos) = bytes[start..]
            .windows(needle.len())
            .position(|window| window == needle)
        {
            let at = start + pos;
            bytes[at..at + needle.len()].copy_from_slice(replacement);
            start = at + needle.len();
            patched += 1;
        }
        assert!(patched >= 2, "expected name in local header + central dir");
        std::fs::write(&bad, &bytes).unwrap();

        expect_invalid(PikgReader::open(&bad, None).await, "duplicate");

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn rejects_appdoc_pair_mismatch_and_schema_violations() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        // 双 APPDOC 不一致：.jwt claims 是另一个文档。
        let other_owner = DID::from_str("did:bns:other").unwrap();
        let other_doc = AppDoc::builder(AppType::Web, "other-app", "9.9.9", "other", &other_owner)
            .web_pkg(
                SubPkgDesc::new("all.web.other-app.other.bns.did#9.9.9")
                    .package_meta_object_id(ObjId::new_by_raw("pkg".to_string(), vec![9; 32])),
            )
            .build()
            .unwrap();
        let other_value = serde_json::to_value(&other_doc).unwrap();
        let header = base64_url_encode(br#"{"alg":"EdDSA"}"#);
        let payload = base64_url_encode(serde_json::to_string(&other_value).unwrap().as_bytes());
        let fake_jwt = format!("{header}.{payload}.c2ln");

        let bad = fixture.dir.join("pair-mismatch.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |_, bytes| Some(bytes),
            vec![(APPDOC_JWT_ENTRY.to_string(), fake_jwt.into_bytes())],
        );
        expect_invalid(
            PikgReader::open(&bad, None).await,
            "different canonical documents",
        );

        // @schema 错误。
        let bad = fixture.dir.join("bad-schema.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |name, bytes| {
                if name == PACKAGE_META_ENTRY {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    value["@schema"] = Value::String("buckyos.pikg.package-meta.v999".to_string());
                    Some(serde_json::to_vec(&value).unwrap())
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "@schema");

        // app_doc_id 不一致。
        let bad = fixture.dir.join("bad-appdoc-id.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |name, bytes| {
                if name == PACKAGE_META_ENTRY {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    value["app_doc_id"] = Value::String(format!("appdoc:{}", "00".repeat(32)));
                    Some(serde_json::to_vec(&value).unwrap())
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "app_doc_id");

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn rejects_package_meta_objid_size_and_digest_corruption() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        // Package Meta 内容被改（key 不再匹配重算 ObjId）。
        let bad = fixture.dir.join("bad-meta-objid.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |name, bytes| {
                if name == PACKAGE_META_ENTRY {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let objects = value["package_objects"].as_object_mut().unwrap();
                    for (_, meta) in objects.iter_mut() {
                        meta["version"] = Value::String("6.6.6".to_string());
                    }
                    Some(serde_json::to_vec(&value).unwrap())
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "recomputed obj id");

        // content_index size 撒谎（结构校验就应抓到，zip declared 不一致）。
        let bad = fixture.dir.join("bad-size.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |name, bytes| {
                if name == PACKAGE_META_ENTRY {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let index = value["content_index"].as_object_mut().unwrap();
                    for (_, entry) in index.iter_mut() {
                        entry["size"] = Value::from(123456u64);
                    }
                    Some(serde_json::to_vec(&value).unwrap())
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "size mismatch");

        // payload 内容被换：结构校验过（size 相同），Verify 必须抓 digest。
        let bad = fixture.dir.join("bad-digest.pikg");
        {
            let payload_size = std::fs::metadata(&fixture.payload_path).unwrap().len();
            let mut replaced = vec![0u8; payload_size as usize];
            replaced[0] = 0xAA;
            rewrite_zip(
                &pikg_path,
                &bad,
                move |name, bytes| {
                    if name == "web.tar.gz" {
                        Some(replaced.clone())
                    } else {
                        Some(bytes)
                    }
                },
                vec![],
            );
        }
        let reader = PikgReader::open(&bad, None).await.unwrap();
        let digest = reader
            .inspection()
            .bundled_content_digests()
            .next()
            .unwrap()
            .to_string();
        let err = reader.verify_content(&digest).await.unwrap_err();
        assert!(err.to_string().contains("digest mismatch"), "{err}");

        // 缺 entry：content_index 指向不存在的路径。
        let bad = fixture.dir.join("missing-entry.pikg");
        rewrite_zip(
            &pikg_path,
            &bad,
            |name, bytes| {
                if name == "web.tar.gz" {
                    None // 删除 payload
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "does not exist");

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn rejects_metadata_bomb() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;

        // APPDOC.json 超过 1MiB。
        let bad = fixture.dir.join("appdoc-bomb.pikg");
        let mut huge =
            serde_json::to_vec(&serde_json::to_value(&fixture.app_doc).unwrap()).unwrap();
        huge.extend(std::iter::repeat(b' ').take((PIKG_MAX_APPDOC_BYTES + 10) as usize));
        rewrite_zip(
            &pikg_path,
            &bad,
            move |name, bytes| {
                if name == APPDOC_JSON_ENTRY {
                    Some(huge.clone())
                } else {
                    Some(bytes)
                }
            },
            vec![],
        );
        expect_invalid(PikgReader::open(&bad, None).await, "exceeds");

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[tokio::test]
    async fn copy_content_streams_and_validates() {
        let fixture = build_fixture();
        let pikg_path = build_valid_pikg(&fixture).await;
        let reader = PikgReader::open(&pikg_path, None).await.unwrap();
        let digest = reader
            .inspection()
            .bundled_content_digests()
            .next()
            .unwrap()
            .to_string();

        let dest = fixture.dir.join("materialized/web.tar.gz");
        reader.copy_content_to_file(&digest, &dest).await.unwrap();
        let copied_digest = sha256_file_hex(&dest).unwrap();
        assert_eq!(format!("sha256:{copied_digest}"), digest);

        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn package_archive_rejects_absolute_parent_and_link_entries() {
        fn archive_with_raw_entry(name: &str) -> Vec<u8> {
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            header.set_mode(0o644);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            builder.append(&header, Cursor::new([1u8])).unwrap();
            let encoder = builder.into_inner().unwrap();
            encoder.finish().unwrap()
        }

        for name in ["/escape", "../escape", "C:/escape", "dir\\escape"] {
            let archive = archive_with_raw_entry(name);
            assert!(validate_package_archive(Cursor::new(archive)).is_err());
        }

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, "escape", "../../outside")
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        let archive = encoder.finish().unwrap();
        assert!(validate_package_archive(Cursor::new(archive)).is_err());
    }

    #[test]
    fn sub_pkg_name_rules() {
        assert!(validate_sub_pkg_name("amd64_docker_image").is_ok());
        assert!(validate_sub_pkg_name("web").is_ok());
        assert!(validate_sub_pkg_name("model.v1-x").is_ok());
        assert!(validate_sub_pkg_name("..").is_err());
        assert!(validate_sub_pkg_name("a/b").is_err());
        assert!(validate_sub_pkg_name("a\\b").is_err());
        assert!(validate_sub_pkg_name("").is_err());
        assert!(validate_sub_pkg_name("中文名").is_err());
    }

    fn base64_url_encode(data: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }
}
