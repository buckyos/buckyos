use crate::{PermissionItem, RdbInstanceConfig};
use ::kRPC::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use name_lib::DID;
use ndn_lib::{BaseContentObject, Curator, NamedObject, ObjId, Reference};
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

pub const APP_DOC_TYPE: &str = "app";
pub const APP_DOC_SCHEMA_VERSION: u32 = 1;
/// App Document Object ID 的 obj type（v0.5 D2 冻结）。
/// ObjId = `appdoc:hex(sha256(JCS(body)))`，与 Package Meta 的 `pkg` 类型区分。
pub const OBJ_TYPE_APP_DOC: &str = "appdoc";

// App安装的时候，需要当前Zone拥有的Capability定义
pub const APP_CAPABILITY_MINI_MEMORY: &str = "memory";
pub const APP_CAPABILITY_MINI_GPU_MEMORY: &str = "gpu.memory";
pub const APP_CAPABILITY_MINI_GPU_TFLOPS: &str = "gpp.tflops";
//app需要部署在接入互联网的系统（不能部署在纯局域网环境)
pub const APP_CAPABILITY_PUBLIC_INTERNET: &str = "internet";
pub const APP_CAPABILITY_AI_LLM: &str = "ai.llm";

//buckyos 支持的应用类型,to_string后填写在app_doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppType {
    Service, // 系统服务
    #[serde(rename = "dapp")]
    AppService, // 应用服务
    Web,     //静态网页
    Agent,   // AI Agent
}

impl TryFrom<&str> for AppType {
    type Error = &'static str;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            "service" => AppType::Service,
            "dapp" => AppType::AppService,
            "web" => AppType::Web,
            "agent" => AppType::Agent,
            _ => return Err("Invalid app doc type"),
        })
    }
}

impl fmt::Display for AppType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            AppType::Service => "service",
            AppType::AppService => "dapp",
            AppType::Web => "web",
            AppType::Agent => "agent",
        };
        write!(f, "{}", value)
    }
}

//AppDoc \ InstallConfig \ ServiceSpec \ InstanceConfig 的基本设计
// App开发者发布的，有签名的Config是 AppDoc （已知应用，其更新应该走did-document的标准机制)
// AppDoc + InstallConfig后，保存在system_config（已安装应用）上的是 [AppServiceSpec],如果应用有更新，必要的时候是需要修改AppServiceSpec来执行更新的
// 调度器基于AppServiceSpec，部署在Node上的是 AppInstanceConfig (这个必然是自动构建的)
//    为了减少多次获取信息的一致性问题，AppInstanceConfig中包含了所有信息（包含AppDoc,InstallConfig)

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct DataMountRecommend {
    pub mount_point: String,
    pub reason: HashMap<String, String>, //key: language_id, value: reason
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ServiceProtocol {
    Http,
    Https,
    Tcp,
    Udp,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceExposeRouteTips {
    Web,
    Port {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preferred_port: Option<u16>,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct ServiceExposeTips {
    pub route: ServiceExposeRouteTips,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub allow_guest: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct ServiceEndpointInfo {
    pub protocol: ServiceProtocol,
    pub inner_port: u16,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub description: HashMap<String, String>, //key: language_id, value: description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose: Option<ServiceExposeTips>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct MountPointInfo {
    //说明挂载点的名称
    pub mount_point_name: String,
    pub access: String, //read_only, read_write, read_write_append
    //详细说明该挂载点的用途
    pub reason: HashMap<String, String>, //key: language_id, value: reason
}
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct BashEnvInfo {
    pub required: bool,
    #[serde(default)]
    pub description: HashMap<String, String>, //key: language_id, value: description
}
/// Governs whether a container App/Agent gets a private instance volume
/// (§7.1 of the paios container spec).
///
/// * `Required` (default): the loader must create an instance volume and will
///   refuse to start the app without one. Fits Script apps, Agents and any
///   service that caches deps or self-evolving state.
/// * `Optional`: an instance volume is created if available but the app can
///   function without one. Reserved for mixed-use cases.
/// * `Disabled`: the app explicitly opts out — used by pure binaries with no
///   private mutable state.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InstanceVolumeMode {
    Required,
    Optional,
    Disabled,
}

impl Default for InstanceVolumeMode {
    fn default() -> Self {
        InstanceVolumeMode::Required
    }
}

/// Developer declaration of instance-volume semantics (§11 of the spec).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstanceVolumeConfig {
    #[serde(default)]
    pub mode: InstanceVolumeMode,
    /// Soft quota in MiB. Not enforced yet (§7.10 observability stub).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_mib: Option<u64>,
    /// Relative paths inside the instance volume whose loss is expected when
    /// the user runs "reset app". Used both for docs and later cleanup tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ephemeral_contents: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct ServiceConfigTips {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub service_endpoints: HashMap<String, ServiceEndpointInfo>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub data_mount_points: HashMap<PathBuf, Option<MountPointInfo>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    //local_cache mount默认需要读写权限
    pub local_cache_mount_points: HashMap<PathBuf, Option<MountPointInfo>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub external_mount_points: HashMap<PathBuf, Option<MountPointInfo>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub rdb_instances: HashMap<String, RdbInstanceConfig>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub instance_volume: InstanceVolumeConfig,
    //bash_envs: key:env_name, value: description<language_id, description>
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub bash_envs: HashMap<String, BashEnvInfo>,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub runtime_caps: HashMap<String, String>, //capname -> "enabled" or "disabled"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_param: Option<String>,

    #[serde(flatten)]
    pub custom_config: HashMap<String, serde_json::Value>,
}

/// 平台选择条件（v0.5 D2）。字段全部可选，省略的维度表示不限制。
/// os/arch 匹配前会做别名归一（amd64=x86_64、arm64=aarch64、darwin=macos 等）。
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct PackageSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_kernel_version: Option<String>,
}

pub fn normalize_arch(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" | "x64" => "x86_64".to_string(),
        "arm64" | "aarch64" => "aarch64".to_string(),
        other => other.to_string(),
    }
}

pub fn normalize_os(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "darwin" | "apple" | "macos" | "osx" => "macos".to_string(),
        "win" | "win32" | "windows" => "windows".to_string(),
        other => other.to_string(),
    }
}

impl PackageSelector {
    pub fn for_platform(os: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            os: Some(normalize_os(os.into().as_str())),
            arch: Some(normalize_arch(arch.into().as_str())),
            min_kernel_version: None,
        }
    }

    /// 是否匹配目标平台。os/arch 只按归一化后的值比较，min_kernel_version 由
    /// planner 结合目标 Node 信息单独校验。
    pub fn matches_platform(&self, os: &str, arch: &str) -> bool {
        let os_ok = self
            .os
            .as_deref()
            .map(|value| normalize_os(value) == normalize_os(os))
            .unwrap_or(true);
        let arch_ok = self
            .arch
            .as_deref()
            .map(|value| normalize_arch(value) == normalize_arch(arch))
            .unwrap_or(true);
        os_ok && arch_ok
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct SubPkgDesc {
    pub pkg_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkg_objid: Option<ObjId>, //PackageMeta的objid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_name: Option<String>, //like buckyos/nightly-buckyos-filebrowser:0.4.1-amd64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_digest: Option<String>, //docker digest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// 显式平台选择条件；省略时已知 key 按 `derived_selector_for_key` 派生，
    /// 未知 key 无显式 selector 时不参与自动选择（v0.5 D2）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<PackageSelector>,
    /// 该 package 对匹配目标是否必需；省略视为 true（v0.5 D2）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

impl SubPkgDesc {
    pub fn get_pkg_id_with_objid(&self) -> Option<String> {
        PackageId::get_pkgid_with_objid(self.pkg_id.as_str(), self.pkg_objid.clone()).ok()
    }

    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(true)
    }
}

/// 已知 pkg_list key 的派生 selector 表（v0.5 D2 冻结的固定命名表）。
/// 返回 None 表示该 key 没有派生规则，必须显式声明 selector 才能被自动选择。
pub fn derived_selector_for_key(key: &str) -> Option<PackageSelector> {
    match key {
        "amd64_docker_image" => Some(PackageSelector::for_platform("linux", "x86_64")),
        "aarch64_docker_image" => Some(PackageSelector::for_platform("linux", "aarch64")),
        "amd64_linux_app" => Some(PackageSelector::for_platform("linux", "x86_64")),
        "aarch64_linux_app" => Some(PackageSelector::for_platform("linux", "aarch64")),
        "amd64_win_app" => Some(PackageSelector::for_platform("windows", "x86_64")),
        "aarch64_win_app" => Some(PackageSelector::for_platform("windows", "aarch64")),
        "amd64_apple_app" => Some(PackageSelector::for_platform("macos", "x86_64")),
        "aarch64_apple_app" => Some(PackageSelector::for_platform("macos", "aarch64")),
        // 平台无关内容：空 selector 匹配所有目标。
        "web" | "agent" | "agent_skills" | "agent_tools" | "script" => {
            Some(PackageSelector::default())
        }
        _ => None,
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct SubPkgList {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_docker_image: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_docker_image: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_linux_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_linux_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_win_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_win_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_apple_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_apple_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_skills: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_tools: Option<SubPkgDesc>,
    #[serde(flatten)]
    pub others: HashMap<String, SubPkgDesc>,
}

impl SubPkgList {
    pub fn get_app_pkg_id(&self) -> Option<String> {
        //根据编译时的目标系统，返回对应的app pkg_id
        if cfg!(target_os = "linux") {
            let pkg = if cfg!(target_arch = "aarch64") {
                self.aarch64_linux_app
                    .as_ref()
                    .or(self.amd64_linux_app.as_ref())
            } else {
                self.amd64_linux_app
                    .as_ref()
                    .or(self.aarch64_linux_app.as_ref())
            };
            if let Some(pkg) = pkg {
                if let Some(pkg_id) = pkg.get_pkg_id_with_objid() {
                    return Some(pkg_id);
                }
            }
            return None;
        } else if cfg!(target_os = "macos") {
            let pkg = if cfg!(target_arch = "aarch64") {
                self.aarch64_apple_app.as_ref()
            } else {
                self.amd64_apple_app.as_ref()
            };
            if let Some(pkg) = pkg {
                if let Some(pkg_id) = pkg.get_pkg_id_with_objid() {
                    return Some(pkg_id);
                }
            }
            return None;
        } else if cfg!(target_os = "windows") {
            let pkg = if cfg!(target_arch = "aarch64") {
                self.aarch64_win_app.as_ref()
            } else {
                self.amd64_win_app.as_ref()
            };
            if let Some(pkg) = pkg {
                if let Some(pkg_id) = pkg.get_pkg_id_with_objid() {
                    return Some(pkg_id);
                }
            }
            return None;
        }

        None
    }

    pub fn get_docker_image_pkg_id(&self) -> Option<String> {
        //根据当前编译期架构，返回对应的docker image pkg_id
        if cfg!(target_arch = "aarch64") {
            if let Some(pkg) = &self.aarch64_docker_image {
                if let Some(pkg_id) = pkg.get_pkg_id_with_objid() {
                    return Some(pkg_id);
                }
            }
        } else if let Some(pkg) = &self.amd64_docker_image {
            if let Some(pkg_id) = pkg.get_pkg_id_with_objid() {
                return Some(pkg_id);
            }
        }

        None
    }
    pub fn get(&self, key: &str) -> Option<&SubPkgDesc> {
        match key {
            "amd64_docker_image" => self.amd64_docker_image.as_ref(),
            "aarch64_docker_image" => self.aarch64_docker_image.as_ref(),
            "amd64_linux_app" => self.amd64_linux_app.as_ref(),
            "aarch64_linux_app" => self.aarch64_linux_app.as_ref(),
            "amd64_win_app" => self.amd64_win_app.as_ref(),
            "aarch64_win_app" => self.aarch64_win_app.as_ref(),
            "aarch64_apple_app" => self.aarch64_apple_app.as_ref(),
            "amd64_apple_app" => self.amd64_apple_app.as_ref(),
            "script" => self.script.as_ref(),
            "web" => self.web.as_ref(),
            "agent" => self.agent.as_ref(),
            "agent_skills" => self.agent_skills.as_ref(),
            "agent_tools" => self.agent_tools.as_ref(),
            _ => self.others.get(key),
        }
    }

    pub fn iter(&self) -> Vec<(String, &SubPkgDesc)> {
        let mut list = Vec::new();
        if let Some(pkg) = &self.amd64_docker_image {
            list.push(("amd64_docker_image".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_docker_image {
            list.push(("aarch64_docker_image".to_string(), pkg));
        }
        if let Some(pkg) = &self.amd64_linux_app {
            list.push(("amd64_linux_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_linux_app {
            list.push(("aarch64_linux_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.amd64_win_app {
            list.push(("amd64_win_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_win_app {
            list.push(("aarch64_win_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_apple_app {
            list.push(("aarch64_apple_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.amd64_apple_app {
            list.push(("amd64_apple_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.script {
            list.push(("script".to_string(), pkg));
        }
        if let Some(pkg) = &self.web {
            list.push(("web".to_string(), pkg));
        }
        if let Some(pkg) = &self.agent {
            list.push(("agent".to_string(), pkg));
        }
        if let Some(pkg) = &self.agent_skills {
            list.push(("agent_skills".to_string(), pkg));
        }
        if let Some(pkg) = &self.agent_tools {
            list.push(("agent_tools".to_string(), pkg));
        }
        for (k, v) in self.others.iter() {
            list.push((k.clone(), v));
        }
        list
    }

    /// 得到某个 entry 的有效 selector：显式声明优先，否则按固定命名表派生。
    /// 返回 None 表示该 entry 不参与自动平台选择。
    pub fn effective_selector(key: &str, desc: &SubPkgDesc) -> Option<PackageSelector> {
        desc.selector
            .clone()
            .or_else(|| derived_selector_for_key(key))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(try_from = "String", into = "String")]
pub enum SelectorType {
    #[default]
    Single,
    Static, //no instance, only one static web page
    Random,
    ByEvent,        //由特定的时间触发运行
    Custom(String), //custom selector type, like "round_robin"
}

impl From<SelectorType> for String {
    fn from(value: SelectorType) -> Self {
        match value {
            SelectorType::Single => "single".into(),
            SelectorType::Static => "static".into(),
            SelectorType::Random => "random".into(),
            SelectorType::ByEvent => "by_event".into(),
            SelectorType::Custom(s) => s,
        }
    }
}

impl TryFrom<String> for SelectorType {
    type Error = &'static str;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Ok(match value.as_str() {
            "single" => SelectorType::Single,
            "static" => SelectorType::Static,
            "random" => SelectorType::Random,
            "by_event" => SelectorType::ByEvent,
            other => SelectorType::Custom(other.to_owned()),
        })
    }
}

/// AppDoc 的 `doc_type` 标记类型：序列化固定为 `"app"`，反序列化拒绝其它取值。
/// 用类型而不是 String，让"doc_type 必填且只能是 app"成为编译期/解析期硬约束。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AppDocType;

impl Serialize for AppDocType {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(APP_DOC_TYPE)
    }
}

impl<'de> Deserialize<'de> for AppDocType {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw == APP_DOC_TYPE {
            Ok(AppDocType)
        } else {
            Err(serde::de::Error::custom(format!(
                "AppDoc doc_type must be `{APP_DOC_TYPE}`, got `{raw}`"
            )))
        }
    }
}

impl fmt::Display for AppDocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(APP_DOC_TYPE)
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct AppPresentation {
    //language_id -> title, summary, description
    pub title: HashMap<String, String>,
    pub summary: HashMap<String, String>,
    pub description: HashMap<String, String>,
    pub icons: HashMap<String, ObjId>,
    pub links: HashMap<String, String>,
    pub license: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppDocWire {
    pub schema_version: u32,
    pub doc_type: AppDocType,
    pub did: DID,
    #[serde(default)]
    pub name: String,
    pub author: DID,
    pub owner: DID,
    pub create_time: u64,
    pub last_update_time: u64,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub base_on: Option<ObjId>,
    #[serde(default)]
    pub directory: HashMap<String, Curator>,
    #[serde(default)]
    pub references: HashMap<String, Reference>,
    pub exp: u64,
    pub version: String,
    #[serde(default)]
    pub version_tag: Option<String>,
    pub app_type: AppType,
    pub controller: DID,
    pub pkg_list: SubPkgList,
    pub show_name: String,
    #[serde(default)]
    pub presentation: Option<AppPresentation>,
    #[serde(default)]
    pub sdk_version: Option<String>,
    #[serde(default)]
    pub req_capbilities: HashMap<String, i64>,
    #[serde(default)]
    pub permissions: Vec<PermissionItem>,
    pub selector_type: SelectorType,
    pub service_config_tips: ServiceConfigTips,
}

//App doc is store at Index-db, publish to bucky store
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct AppDoc {
    #[serde(flatten)]
    pub _base: BaseContentObject,
    pub schema_version: u32,
    pub doc_type: AppDocType,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_tag: Option<String>,
    pub app_type: AppType,
    pub controller: DID,
    pub pkg_list: SubPkgList,

    pub show_name: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<AppPresentation>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdk_version: Option<String>, //使用哪个版本的 buckyos sdk版本开发，如果未设置则为兼容App,AppLoader要使用兼容模式启动Docker
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub req_capbilities: HashMap<String, i64>, //key: capability_name, value: required capability_value
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionItem>,

    //UI 应该根据service_config_tips的提示，来构造UI，得到最终的ServiceConfig
    pub selector_type: SelectorType,
    pub service_config_tips: ServiceConfigTips,
}

impl<'de> Deserialize<'de> for AppDoc {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppDocWire::deserialize(deserializer)?;
        Ok(Self {
            _base: BaseContentObject {
                did: Some(wire.did),
                name: wire.name,
                author: wire.author.to_string(),
                owner: wire.owner,
                create_time: wire.create_time,
                last_update_time: wire.last_update_time,
                copyright: wire.copyright,
                tags: wire.tags,
                categories: wire.categories,
                base_on: wire.base_on,
                directory: wire.directory,
                references: wire.references,
                exp: wire.exp,
            },
            schema_version: wire.schema_version,
            doc_type: wire.doc_type,
            version: wire.version,
            version_tag: wire.version_tag,
            app_type: wire.app_type,
            controller: wire.controller,
            pkg_list: wire.pkg_list,
            show_name: wire.show_name,
            presentation: wire.presentation,
            sdk_version: wire.sdk_version,
            req_capbilities: wire.req_capbilities,
            permissions: wire.permissions,
            selector_type: wire.selector_type,
            service_config_tips: wire.service_config_tips,
        })
    }
}

impl Deref for AppDoc {
    type Target = BaseContentObject;

    fn deref(&self) -> &Self::Target {
        &self._base
    }
}

impl DerefMut for AppDoc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self._base
    }
}

impl NamedObject for AppDoc {
    fn get_obj_type() -> &'static str {
        OBJ_TYPE_APP_DOC
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct AppDocSignatureEnvelope {
    pub schema_version: u32,
    pub app_doc_object_id: ObjId,
    pub signer: DID,
    pub controller: DID,
    pub verification_method: String,
    pub algorithm: String,
    pub signature: String,
}

impl AppDocSignatureEnvelope {
    pub fn validate_for(&self, app_doc: &AppDoc) -> Result<()> {
        app_doc.validate()?;
        let (object_id, _) = app_doc.gen_obj_id();
        let author = app_doc.author_did()?;
        if self.schema_version != APP_DOC_SCHEMA_VERSION
            || self.app_doc_object_id != object_id
            || self.signer != author
            || self.controller != app_doc.controller
            || !self
                .verification_method
                .starts_with(&format!("{}#", self.controller.to_string()))
            || self.algorithm != "EdDSA"
            || self.signature.is_empty()
        {
            return Err(RPCErrors::ReasonError(
                "AppDoc signature envelope does not match the AppDoc authority".to_string(),
            ));
        }
        Ok(())
    }

    pub fn verify_for(&self, app_doc: &AppDoc, verifying_key: &[u8; 32]) -> Result<()> {
        self.validate_for(app_doc)?;
        let verifying_key = VerifyingKey::from_bytes(verifying_key).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid AppDoc Ed25519 verifying key: {error}"))
        })?;
        let signature_bytes = URL_SAFE_NO_PAD.decode(&self.signature).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid AppDoc base64url signature: {error}"))
        })?;
        let signature = Signature::from_slice(&signature_bytes).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid AppDoc Ed25519 signature: {error}"))
        })?;
        let (_, canonical_body) = app_doc.gen_obj_id();
        verifying_key
            .verify(canonical_body.as_bytes(), &signature)
            .map_err(|_| RPCErrors::ReasonError("AppDoc signature verification failed".to_string()))
    }
}

impl AppDoc {
    pub fn app_did(&self) -> &DID {
        self.did.as_ref().expect("AppDoc.did must be present")
    }

    pub fn author_did(&self) -> Result<DID> {
        DID::from_str(&self.author).map_err(|error| {
            RPCErrors::ReasonError(format!("AppDoc author must be a valid DID: {error}"))
        })
    }

    /// 按 v0.5 冻结的确定性命名规则构造 App DID：`did:bns:{app_name}.{owner_id}`。
    /// 这是"名字结构的确定性结果"，与 candidate 自声明 owner 建立信任是两回事；
    /// 不适用该规则的应用必须显式提供 App DID。
    pub fn derive_bns_app_did(app_name: &str, owner: &DID) -> Result<DID> {
        let app_name = app_name.trim();
        if app_name.is_empty() {
            return Err(RPCErrors::ReasonError(
                "derive app did failed: app name is empty".to_string(),
            ));
        }
        if owner.id.trim().is_empty() || owner.id == "undefined" {
            return Err(RPCErrors::ReasonError(
                "derive app did failed: owner did is invalid".to_string(),
            ));
        }
        Ok(DID::new(
            "bns",
            format!("{}.{}", app_name, owner.id).as_str(),
        ))
    }

    pub fn get_app_type(&self) -> AppType {
        self.app_type
    }

    pub fn app_icon_url(&self) -> Option<&str> {
        self.presentation
            .as_ref()
            .and_then(|presentation| presentation.links.get("app_icon_url"))
            .map(String::as_str)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != APP_DOC_SCHEMA_VERSION {
            return Err(RPCErrors::ReasonError(format!(
                "unsupported AppDoc schema_version {}",
                self.schema_version
            )));
        }
        let app_did = self.did.as_ref().ok_or_else(|| {
            RPCErrors::ReasonError("AppDoc BaseContentObject.did is required".to_string())
        })?;
        let app_id = crate::AppId::from_app_did(app_did).map_err(RPCErrors::ReasonError)?;
        let author = self.author_did()?;
        if self.version.trim().is_empty()
            || self.show_name.trim().is_empty()
            || author.is_undefined()
            || self.owner.is_undefined()
            || self.controller.is_undefined()
            || self.exp == 0
            || self.exp < self.create_time
            || self.last_update_time < self.create_time
        {
            return Err(RPCErrors::ReasonError(
                "AppDoc contains invalid required envelope/body fields".to_string(),
            ));
        }
        if self.app_type != AppType::Service && self.pkg_list.iter().is_empty() {
            return Err(RPCErrors::ReasonError(
                "non-system AppDoc pkg_list must contain at least one package".to_string(),
            ));
        }
        let namespace_suffix = format!(".{app_id}");
        for (sub_pkg_name, package) in self.pkg_list.iter() {
            let parsed = PackageId::parse(&package.pkg_id).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "AppDoc package `{sub_pkg_name}` has invalid PackageId: {error}"
                ))
            })?;
            let unique_name = parsed.get_unique_name().map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "AppDoc package `{sub_pkg_name}` has invalid package name: {error}"
                ))
            })?;
            let unique_name = unique_name.strip_prefix("all.").unwrap_or(&unique_name);
            let is_root_package = unique_name == app_id.as_str();
            let is_sub_package = unique_name
                .strip_suffix(&namespace_suffix)
                .is_some_and(|prefix| !prefix.is_empty() && !prefix.contains('.'));
            if !is_root_package && !is_sub_package {
                return Err(RPCErrors::ReasonError(format!(
                    "AppDoc package `{sub_pkg_name}` is outside AppId namespace `{app_id}`"
                )));
            }
            if parsed
                .version_exp
                .as_ref()
                .map(ToString::to_string)
                .as_deref()
                != Some(self.version.as_str())
                || package.pkg_objid.is_none()
            {
                return Err(RPCErrors::ReasonError(format!(
                    "AppDoc package `{sub_pkg_name}` must pin App version and Package Meta ObjectId"
                )));
            }
        }
        Ok(())
    }

    pub fn validate_resolved_identity(
        &self,
        expected_did: &DID,
        expected_object_id: &ObjId,
    ) -> Result<()> {
        self.validate()?;
        let (actual_object_id, _) = self.gen_obj_id();
        if self.app_did() != expected_did || &actual_object_id != expected_object_id {
            return Err(RPCErrors::ReasonError(
                "resolved AppDID/AppDoc ObjectId does not match the AppDoc snapshot".to_string(),
            ));
        }
        Ok(())
    }
}

//实现一个AppDoc builder,可以使用链式调用创建app_doc
// 1) builder首先要指定AppType
// 2) 根据type的不同，可以做不同的初始化操作
// 3）最后build构造AppDoc时，如果缺少必要的字段，则需要提示用户，并给出建议
// 基本限制
//   Service: SubPkg必然没有web,也没有docker
//   AppService: SubPkg必然有docker,但没有web,也没有*_app，一般需要申请data目录和cache目录的读写权限，申请library目录的读权限
//   Web: SubPkg必然有web,但没有docker,也没有*_app，不需要任何权限，SelectType为Static
//   Agent: SubPkg必然有agent,可选agent_skills,但没有docker,也没有web/原生app

impl SubPkgDesc {
    pub fn new(pkg_id: impl Into<String>) -> Self {
        Self {
            pkg_id: pkg_id.into(),
            pkg_objid: None,
            docker_image_name: None,
            docker_image_digest: None,
            source_url: None,
            selector: None,
            required: None,
        }
    }

    pub fn selector(mut self, selector: PackageSelector) -> Self {
        self.selector = Some(selector);
        self
    }

    pub fn package_meta_object_id(mut self, object_id: ObjId) -> Self {
        self.pkg_objid = Some(object_id);
        self
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = Some(required);
        self
    }

    pub fn docker_image_name(mut self, docker_image_name: impl Into<String>) -> Self {
        self.docker_image_name = Some(docker_image_name.into());
        self
    }

    pub fn docker_image_digest(mut self, docker_image_digest: impl Into<String>) -> Self {
        self.docker_image_digest = Some(docker_image_digest.into());
        self
    }

    pub fn source_url(mut self, source_url: impl Into<String>) -> Self {
        self.source_url = Some(source_url.into());
        self
    }
}

pub struct AppDocBuilder {
    app_type: AppType,
    app_name: String,
    app_did: Option<DID>,
    owner: DID,
    controller: DID,
    author: DID,
    version: String,
    version_tag: Option<String>,
    create_time: u64,
    last_update_time: u64,
    exp: u64,
    show_name: Option<String>,
    presentation: Option<AppPresentation>,
    sdk_version: Option<String>,
    req_capbilities: HashMap<String, i64>,
    permissions: Vec<PermissionItem>,
    selector_type: Option<SelectorType>,
    service_config_tips: ServiceConfigTips,
    pkg_list: SubPkgList,
    apply_default_permissions: bool,
}

impl AppDocBuilder {
    /// Build a new AppDoc with minimal required meta fields.
    ///
    /// Notes:
    /// - `AppType` will be written into `categories[0]`.
    /// - `create_time` and `last_update_time` default to current unix seconds.
    pub fn new(
        app_type: AppType,
        name: impl Into<String>,
        version: impl Into<String>,
        author: impl Into<String>,
        owner: &DID,
    ) -> Self {
        let now = buckyos_kit::buckyos_get_unix_timestamp();
        let name = name.into();
        let version = version.into();
        let author = author.into();
        let author = if author.starts_with("did:") {
            DID::from_str(&author).unwrap_or_else(|_| DID::undefined())
        } else {
            DID::new("bns", &author)
        };

        Self {
            app_type,
            app_name: name,
            app_did: None,
            owner: owner.clone(),
            controller: owner.clone(),
            author,
            version,
            version_tag: None,
            create_time: now,
            last_update_time: now,
            exp: now + 3600 * 24 * 365,
            show_name: None,
            presentation: None,
            sdk_version: None,
            req_capbilities: HashMap::new(),
            permissions: vec![],
            selector_type: None,
            service_config_tips: ServiceConfigTips::default(),
            pkg_list: SubPkgList::default(),
            apply_default_permissions: true,
        }
    }

    /// 显式指定 App DID；不调用时 build() 按冻结规则
    /// `did:bns:{app_name}.{owner_id}` 构造（见 `AppDoc::derive_bns_app_did`）。
    pub fn app_did(mut self, app_did: DID) -> Self {
        self.app_did = Some(app_did);
        self
    }

    pub fn show_name(mut self, show_name: impl Into<String>) -> Self {
        self.show_name = Some(show_name.into());
        self
    }

    pub fn app_icon_url(mut self, app_icon_url: impl Into<String>) -> Self {
        self.presentation
            .get_or_insert_with(AppPresentation::default)
            .links
            .insert("app_icon_url".to_string(), app_icon_url.into());
        self
    }

    pub fn presentation(mut self, presentation: AppPresentation) -> Self {
        self.presentation = Some(presentation);
        self
    }

    pub fn sdk_version(mut self, sdk_version: impl Into<String>) -> Self {
        self.sdk_version = Some(sdk_version.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.version_tag = Some(tag.into());
        self
    }

    pub fn exp(mut self, exp: u64) -> Self {
        self.exp = exp;
        self
    }

    pub fn controller(mut self, controller: DID) -> Self {
        self.controller = controller;
        self
    }

    /// Set i18n description detail text.
    ///
    /// Stored into `PackageMeta.meta["description"]` in the following form:
    /// `{ "detail": { "<language_id>": "<text>" } }`.
    pub fn description(mut self, language_id: impl Into<String>, text: impl Into<String>) -> Self {
        let language_id = language_id.into();
        let text = text.into();

        self.presentation
            .get_or_insert_with(AppPresentation::default)
            .description
            .insert(language_id, text);
        self
    }

    /// Advanced: set raw `PackageMeta.meta["description"]` value directly.
    pub fn description_raw(mut self, description: serde_json::Value) -> Self {
        if let Some(values) = description
            .get("detail")
            .and_then(serde_json::Value::as_object)
        {
            let target = &mut self
                .presentation
                .get_or_insert_with(AppPresentation::default)
                .description;
            for (language, value) in values {
                if let Some(text) = value.as_str() {
                    target.insert(language.clone(), text.to_string());
                }
            }
        }
        self
    }

    pub fn description_detail(self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        // Backward-compatible helper: write a single-language "en" description.
        self.description("en", detail)
    }

    pub fn selector_type(mut self, selector_type: SelectorType) -> Self {
        self.selector_type = Some(selector_type);
        self
    }

    pub fn req_capability(mut self, name: impl Into<String>, value: i64) -> Self {
        self.req_capbilities.insert(name.into(), value);
        self
    }

    pub fn add_permission(mut self, permission: PermissionItem) -> Self {
        self.permissions.push(permission);
        self
    }

    pub fn apply_default_permissions(mut self, apply: bool) -> Self {
        self.apply_default_permissions = apply;
        self
    }

    // -------- install tips helpers --------
    pub fn add_data_mount_point(mut self, mount_point: impl Into<String>) -> Self {
        self.service_config_tips
            .data_mount_points
            .insert(PathBuf::from(mount_point.into()), None);
        self
    }

    pub fn add_local_cache_mount_point(mut self, mount_point: impl Into<String>) -> Self {
        self.service_config_tips
            .local_cache_mount_points
            .insert(PathBuf::from(mount_point.into()), None);
        self
    }

    pub fn service_port(mut self, service_name: impl Into<String>, port: u16) -> Self {
        let service_name = service_name.into();
        let (protocol, route) = if service_name == "www" {
            (ServiceProtocol::Http, ServiceExposeRouteTips::Web)
        } else {
            (
                ServiceProtocol::Tcp,
                ServiceExposeRouteTips::Port {
                    preferred_port: Some(port),
                },
            )
        };
        self.service_config_tips.service_endpoints.insert(
            service_name,
            ServiceEndpointInfo {
                protocol,
                inner_port: port,
                required: true,
                description: HashMap::new(),
                expose: Some(ServiceExposeTips {
                    route,
                    scope: String::new(),
                    allow_guest: false,
                }),
            },
        );
        self
    }

    pub fn container_param(mut self, param: impl Into<String>) -> Self {
        self.service_config_tips.container_param = Some(param.into());
        self
    }

    pub fn start_param(mut self, param: impl Into<String>) -> Self {
        self.service_config_tips.start_param = Some(param.into());
        self
    }

    pub fn install_custom(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.service_config_tips
            .custom_config
            .insert(key.into(), value);
        self
    }

    // -------- sub packages helpers --------
    pub fn amd64_docker_image(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.amd64_docker_image = Some(desc);
        self
    }

    pub fn aarch64_docker_image(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.aarch64_docker_image = Some(desc);
        self
    }

    pub fn amd64_linux_app(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.amd64_linux_app = Some(desc);
        self
    }

    pub fn aarch64_linux_app(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.aarch64_linux_app = Some(desc);
        self
    }

    pub fn script_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.script = Some(desc);
        self
    }

    pub fn web_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.web = Some(desc);
        self
    }

    pub fn agent_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.agent = Some(desc);
        self
    }

    pub fn agent_skills_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.agent_skills = Some(desc);
        self
    }

    pub fn agent_tools_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.agent_tools = Some(desc);
        self
    }

    pub fn other_pkg(mut self, key: impl Into<String>, desc: SubPkgDesc) -> Self {
        self.pkg_list.others.insert(key.into(), desc);
        self
    }

    fn ensure_appservice_default_permissions(&mut self) {
        if !self.apply_default_permissions || !self.permissions.is_empty() {
            return;
        }

        self.permissions.push(PermissionItem {
            scope_path: "fs.data".to_string(),
            required: true,
            actions: vec!["read".to_string(), "write".to_string()],
            exp: None,
        });
        self.permissions.push(PermissionItem {
            scope_path: "fs.cache".to_string(),
            required: true,
            actions: vec!["read".to_string(), "write".to_string()],
            exp: None,
        });
        self.permissions.push(PermissionItem {
            scope_path: "fs.library".to_string(),
            required: false,
            actions: vec!["read".to_string()],
            exp: None,
        });
    }

    pub fn build(mut self) -> Result<AppDoc> {
        let has_docker = self.pkg_list.amd64_docker_image.is_some()
            || self.pkg_list.aarch64_docker_image.is_some();
        let has_script = self.pkg_list.script.is_some();
        let has_web = self.pkg_list.web.is_some();
        let has_native_app = self.pkg_list.amd64_linux_app.is_some()
            || self.pkg_list.aarch64_linux_app.is_some()
            || self.pkg_list.amd64_win_app.is_some()
            || self.pkg_list.aarch64_win_app.is_some()
            || self.pkg_list.amd64_apple_app.is_some()
            || self.pkg_list.aarch64_apple_app.is_some();
        let has_agent = self.pkg_list.agent.is_some();
        let has_agent_skills = self.pkg_list.agent_skills.is_some();
        let has_agent_tools = self.pkg_list.agent_tools.is_some();

        let mut errors: Vec<String> = vec![];
        match self.app_type {
            AppType::Service => {
                if has_web {
                    errors.push(
                        "Service app must not include `pkg_list.web` (remove it or change AppType)"
                            .to_string(),
                    );
                }
                if has_docker {
                    errors.push("Service app must not include docker images (remove `amd64_docker_image`/`aarch64_docker_image` or change AppType)".to_string());
                }
                if has_agent || has_agent_skills || has_agent_tools {
                    errors.push(
                        "Service app must not include Agent packages (remove `pkg_list.agent`, `pkg_list.agent_skills`, and `pkg_list.agent_tools` or change AppType)"
                            .to_string(),
                    );
                }
            }
            AppType::AppService => {
                if !has_docker && !has_script {
                    errors.push("AppService app must include a runtime package (set `pkg_list.script`, `amd64_docker_image`, and/or `aarch64_docker_image`)".to_string());
                }
                if has_web {
                    errors.push(
                        "AppService app must not include `pkg_list.web` (remove it or change AppType)"
                            .to_string(),
                    );
                }
                if has_native_app {
                    errors.push("AppService app must not include `*_win_app`/`*_apple_app` packages (remove them or change AppType)".to_string());
                }
                if has_agent || has_agent_skills || has_agent_tools {
                    errors.push(
                        "AppService app must not include Agent packages (remove `pkg_list.agent`, `pkg_list.agent_skills`, and `pkg_list.agent_tools` or change AppType)"
                            .to_string(),
                    );
                }
                self.ensure_appservice_default_permissions();
            }
            AppType::Web => {
                if !has_web {
                    errors.push("Web app must include `pkg_list.web`".to_string());
                }
                if has_docker {
                    errors.push(
                        "Web app must not include docker images (remove them or change AppType)"
                            .to_string(),
                    );
                }
                if has_native_app {
                    errors.push(
                        "Web app must not include native app packages (remove them or change AppType)"
                            .to_string(),
                    );
                }
                if has_agent || has_agent_skills || has_agent_tools {
                    errors.push(
                        "Web app must not include Agent packages (remove `pkg_list.agent`, `pkg_list.agent_skills`, and `pkg_list.agent_tools` or change AppType)"
                            .to_string(),
                    );
                }

                // Web is always static and should not request permissions.
                self.selector_type = Some(SelectorType::Static);
                self.permissions.clear();
                self.service_config_tips = ServiceConfigTips::default();
            }
            AppType::Agent => {
                if !has_agent {
                    errors.push("Agent app must include `pkg_list.agent`".to_string());
                }
                if has_docker {
                    errors.push(
                        "Agent app must not include docker images (remove them or change AppType)"
                            .to_string(),
                    );
                }
                if has_web {
                    errors.push(
                        "Agent app must not include `pkg_list.web` (remove it or change AppType)"
                            .to_string(),
                    );
                }
                if has_native_app {
                    errors.push(
                        "Agent app must not include native app packages (remove them or change AppType)"
                            .to_string(),
                    );
                }
            }
        }

        if !errors.is_empty() {
            return Err(RPCErrors::ReasonError(errors.join("; ")));
        }

        let app_did = match self.app_did.clone() {
            Some(did) => did,
            None => AppDoc::derive_bns_app_did(self.app_name.as_str(), &self.owner)?,
        };

        let show_name = self
            .show_name
            .clone()
            .or_else(|| Some(self.app_name.clone()))
            .unwrap_or_else(|| "Unnamed App".to_string());
        let presentation = self.presentation.or_else(|| {
            Some(AppPresentation {
                description: HashMap::from([("en".to_string(), show_name.clone())]),
                ..Default::default()
            })
        });

        let mut base = BaseContentObject::new(self.app_name);
        base.did = Some(app_did);
        base.author = self.author.to_string();
        base.owner = self.owner;
        base.create_time = self.create_time;
        base.last_update_time = self.last_update_time;
        base.categories.push(self.app_type.to_string());
        base.exp = self.exp;

        let app_doc = AppDoc {
            _base: base,
            schema_version: APP_DOC_SCHEMA_VERSION,
            doc_type: AppDocType,
            version: self.version,
            version_tag: self.version_tag,
            app_type: self.app_type,
            controller: self.controller,
            show_name,
            presentation,
            sdk_version: self.sdk_version,
            req_capbilities: self.req_capbilities,
            permissions: self.permissions,
            selector_type: self.selector_type.unwrap_or_default(),
            service_config_tips: self.service_config_tips,
            pkg_list: self.pkg_list,
        };
        app_doc.validate()?;
        Ok(app_doc)
    }
}

impl AppDoc {
    pub fn builder(
        app_type: AppType,
        name: impl Into<String>,
        version: impl Into<String>,
        author: impl Into<String>,
        owner: &DID,
    ) -> AppDocBuilder {
        AppDocBuilder::new(app_type, name, version, author, owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pinned_package(pkg_id: &str) -> SubPkgDesc {
        SubPkgDesc::new(pkg_id)
            .package_meta_object_id(ObjId::new_by_raw("pkg".to_string(), vec![1; 32]))
    }

    #[test]
    fn required_endpoint_does_not_imply_exposure() {
        let endpoint = ServiceEndpointInfo {
            protocol: ServiceProtocol::Tcp,
            inner_port: 445,
            required: true,
            description: HashMap::new(),
            expose: None,
        };

        let value = serde_json::to_value(&endpoint).unwrap();
        assert!(value.get("description").is_none());
        let decoded: ServiceEndpointInfo = serde_json::from_value(value).unwrap();
        assert!(decoded.required);
        assert!(decoded.expose.is_none());
    }

    #[test]
    fn appdoc_v1_golden_fixture_and_signature_are_stable() {
        let source = include_str!("../../../../doc/fixtures/appdoc-v1.json");
        let expected_jcs = include_str!("../../../../doc/fixtures/appdoc-v1.jcs").trim_end();
        let expected_id = include_str!("../../../../doc/fixtures/appdoc-v1.object-id").trim();
        let envelope_source =
            include_str!("../../../../doc/fixtures/appdoc-v1.signature-envelope.json");
        let verifying_key = URL_SAFE_NO_PAD
            .decode(include_str!("../../../../doc/fixtures/appdoc-v1.verifying-key").trim())
            .unwrap();
        let verifying_key: [u8; 32] = verifying_key.try_into().unwrap();

        let app_doc: AppDoc = serde_json::from_str(source).unwrap();
        let envelope: AppDocSignatureEnvelope = serde_json::from_str(envelope_source).unwrap();
        let (object_id, canonical) = app_doc.gen_obj_id();
        assert_eq!(canonical, expected_jcs);
        assert_eq!(object_id.to_string(), expected_id);
        envelope.verify_for(&app_doc, &verifying_key).unwrap();

        let mut wrong_controller = envelope.clone();
        wrong_controller.controller = DID::from_str("did:web:other.example").unwrap();
        assert!(wrong_controller
            .verify_for(&app_doc, &verifying_key)
            .is_err());
        let mut wrong_signer = envelope.clone();
        wrong_signer.signer = DID::from_str("did:web:other.example").unwrap();
        assert!(wrong_signer.verify_for(&app_doc, &verifying_key).is_err());
        let mut wrong_signature = envelope;
        wrong_signature.signature.replace_range(..1, "A");
        assert!(wrong_signature
            .verify_for(&app_doc, &verifying_key)
            .is_err());
    }

    #[test]
    fn rootfs_appdoc_fixture_follows_v1_schema() {
        let source =
            include_str!("../../../rootfs/local/did_docs/filebrowser.buckyos.bns.did.doc.json");
        let app_doc: AppDoc = serde_json::from_str(source).unwrap();
        app_doc.validate().unwrap();
    }

    #[test]
    fn appdoc_v1_accepts_base_content_metadata_and_rejects_package_meta_fields() {
        let source = include_str!("../../../../doc/fixtures/appdoc-v1.json");
        let value: serde_json::Value = serde_json::from_str(source).unwrap();

        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("controller");
        assert!(serde_json::from_value::<AppDoc>(missing).is_err());

        let mut unknown = value.clone();
        unknown["unexpected"] = json!(true);
        assert!(serde_json::from_value::<AppDoc>(unknown).is_err());

        let mut content = value.clone();
        content["name"] = json!("portable-app-release");
        content["copyright"] = json!("Copyright 2026 Example");
        content["tags"] = json!(["productivity", "web"]);
        content["categories"] = json!(["web"]);
        content["base_on"] = json!(ObjId::new_by_raw("appdoc".to_string(), vec![7; 32]));
        content["directory"] = json!({"catalog": {}});
        content["references"] = json!({"homepage": {}});
        let content_doc: AppDoc = serde_json::from_value(content.clone()).unwrap();
        content_doc.validate().unwrap();
        assert_eq!(content_doc.name, "portable-app-release");
        assert_eq!(content_doc.tags, vec!["productivity", "web"]);
        assert_eq!(serde_json::to_value(&content_doc).unwrap(), content);
        let plain_doc: AppDoc = serde_json::from_value(value.clone()).unwrap();
        assert_ne!(content_doc.gen_obj_id().0, plain_doc.gen_obj_id().0);

        let mut package_meta = value.clone();
        package_meta["deps"] = json!({});
        package_meta["size"] = json!(0);
        package_meta["content"] = json!("");
        assert!(serde_json::from_value::<AppDoc>(package_meta).is_err());

        let app_doc: AppDoc = serde_json::from_value(value).unwrap();
        let (object_id, _) = app_doc.gen_obj_id();
        assert!(app_doc
            .validate_resolved_identity(
                &DID::from_str("did:web:other.example").unwrap(),
                &object_id,
            )
            .is_err());
        assert!(app_doc
            .validate_resolved_identity(
                app_doc.app_did(),
                &ObjId::new_by_raw("appdoc".to_string(), vec![9; 32]),
            )
            .is_err());
    }

    #[test]
    fn test_app_doc_builder_web_enforces_static_and_no_permissions() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let doc = AppDoc::builder(
            AppType::Web,
            "demo-web",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .show_name("Demo Web")
        .description("en", "Demo Web Description")
        .description("zh", "演示网页应用描述")
        .web_pkg(pinned_package("all.web.demo-web.example.com.bns.did#0.1.0"))
        .add_permission(PermissionItem {
            scope_path: "fs.data".to_string(),
            required: true,
            actions: vec!["read".to_string()],
            exp: None,
        })
        .build()
        .unwrap();

        println!(
            "built web app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );

        assert_eq!(doc.selector_type, SelectorType::Static);
        assert!(doc.permissions.is_empty());
        assert_eq!(doc.name, "demo-web");
        assert_eq!(doc.categories, vec!["web"]);

        let sys_testdoc = r#"
{
  "did": "did:bns:buckyos_systest.buckyos.ai",
    "doc_type": "app",
    "name": "buckyos_systest",
    "version": "0.5.1",
    "meta": {
      "detail": "BuckyOS System Test App"
    },
    "create_time": 1743008063,
    "last_update_time": 1743008063,
    "exp": 1837616063,
    "tag": "latest",
    "author": "did:web:buckyos.ai",
    "owner": "did:web:buckyos.ai",
    "show_name": "BuckyOS System Test",
    "selector_type": "static",
    "service_config_tips": {
    },
    "pkg_list": {
      "web": {
        "pkg_id": "nightly-linux-amd64.buckyos_systest#0.5.1"
      }
    }
  }     
"#;
        assert!(serde_json::from_str::<AppDoc>(sys_testdoc).is_err());
    }

    #[test]
    fn test_app_doc_builder_service_minimal_ok_and_rejects_docker_web() {
        let owner = DID::from_str("did:web:example.com").unwrap();

        // A service must carry executable content, but no docker or web package.
        let doc = AppDoc::builder(
            AppType::Service,
            "demo-service",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .show_name("Demo Service")
        .sdk_version("0.5.1")
        .script_pkg(pinned_package(
            "all.script.demo-service.example.com.bns.did#0.1.0",
        ))
        .build()
        .unwrap();
        println!(
            "built service app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );
        assert_eq!(doc.get_app_type(), AppType::Service);
        assert_eq!(doc.sdk_version.as_deref(), Some("0.5.1"));

        // Service must reject docker image.
        let err = AppDoc::builder(
            AppType::Service,
            "demo-service-bad",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .amd64_docker_image(SubPkgDesc::new("demo_service_bad-img#0.1.0"))
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("must not include docker images"),
            "unexpected error: {:?}",
            err
        );

        // Service must reject web package.
        let err = AppDoc::builder(
            AppType::Service,
            "demo-service-bad2",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .web_pkg(SubPkgDesc::new("demo_service_bad2-web#0.1.0"))
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("must not include `pkg_list.web`"),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn test_app_doc_builder_appservice_requires_runtime_and_sets_default_permissions() {
        let owner = DID::from_str("did:web:example.com").unwrap();

        // AppService must require a runnable package.
        let err = AppDoc::builder(
            AppType::AppService,
            "demo-dapp-bad",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("must include a runtime package"),
            "unexpected error: {:?}",
            err
        );

        // AppService should build with docker and auto-fill default permissions when not provided.
        let doc = AppDoc::builder(
            AppType::AppService,
            "demo-dapp",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .amd64_docker_image(
            pinned_package("nightly-linux-amd64.image.demo-dapp.example.com.bns.did#0.1.0")
                .docker_image_name("buckyos/demo_dapp:0.1.0"),
        )
        .build()
        .unwrap();

        println!(
            "built appservice app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );
        assert_eq!(doc.get_app_type(), AppType::AppService);

        let scopes: Vec<&str> = doc
            .permissions
            .iter()
            .map(|p| p.scope_path.as_str())
            .collect();
        assert!(
            scopes.contains(&"fs.data"),
            "permissions: {:?}",
            doc.permissions
        );
        assert!(
            scopes.contains(&"fs.cache"),
            "permissions: {:?}",
            doc.permissions
        );
        assert!(
            scopes.contains(&"fs.library"),
            "permissions: {:?}",
            doc.permissions
        );

        let script_doc = AppDoc::builder(
            AppType::AppService,
            "demo-script-dapp",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .script_pkg(pinned_package(
            "all.script.demo-script-dapp.example.com.bns.did#0.1.0",
        ))
        .build()
        .unwrap();
        assert!(script_doc.pkg_list.script.is_some());
        assert!(script_doc
            .pkg_list
            .iter()
            .iter()
            .any(|(key, _)| key == "script"));
        assert_eq!(script_doc.get_app_type(), AppType::AppService);
    }

    #[test]
    fn test_app_doc_builder_web_requires_web_pkg() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let err = AppDoc::builder(
            AppType::Web,
            "demo-web-bad",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("Web app must include `pkg_list.web`"),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn test_subpkg_list_get_and_iter_cover_all_known_keys() {
        let mut pkg_list = SubPkgList {
            amd64_docker_image: Some(SubPkgDesc::new("demo-img-amd64#0.1.0")),
            aarch64_docker_image: Some(SubPkgDesc::new("demo-img-aarch64#0.1.0")),
            amd64_linux_app: Some(SubPkgDesc::new("demo-linux-amd64#0.1.0")),
            aarch64_linux_app: Some(SubPkgDesc::new("demo-linux-aarch64#0.1.0")),
            amd64_win_app: Some(SubPkgDesc::new("demo-win-amd64#0.1.0")),
            aarch64_win_app: Some(SubPkgDesc::new("demo-win-aarch64#0.1.0")),
            aarch64_apple_app: Some(SubPkgDesc::new("demo-mac-aarch64#0.1.0")),
            amd64_apple_app: Some(SubPkgDesc::new("demo-mac-amd64#0.1.0")),
            web: Some(SubPkgDesc::new("demo-web#0.1.0")),
            agent: Some(SubPkgDesc::new("demo-agent#0.1.0")),
            agent_skills: Some(SubPkgDesc::new("demo-agent-skills#0.1.0")),
            agent_tools: Some(SubPkgDesc::new("demo-agent-tools#0.1.0")),
            ..Default::default()
        };
        pkg_list
            .others
            .insert("custom".to_string(), SubPkgDesc::new("demo-custom#0.1.0"));

        assert_eq!(
            pkg_list
                .get("amd64_linux_app")
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-linux-amd64#0.1.0")
        );
        assert_eq!(
            pkg_list
                .get("aarch64_win_app")
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-win-aarch64#0.1.0")
        );
        assert_eq!(
            pkg_list
                .get("amd64_apple_app")
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-mac-amd64#0.1.0")
        );
        assert_eq!(
            pkg_list.get("agent").map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-agent#0.1.0")
        );
        assert_eq!(
            pkg_list.get("agent_skills").map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-agent-skills#0.1.0")
        );
        assert_eq!(
            pkg_list.get("agent_tools").map(|pkg| pkg.pkg_id.as_str()),
            Some("demo-agent-tools#0.1.0")
        );

        let keys: Vec<String> = pkg_list.iter().into_iter().map(|(key, _)| key).collect();
        assert!(keys.iter().any(|key| key == "amd64_linux_app"));
        assert!(keys.iter().any(|key| key == "aarch64_win_app"));
        assert!(keys.iter().any(|key| key == "amd64_apple_app"));
        assert!(keys.iter().any(|key| key == "agent"));
        assert!(keys.iter().any(|key| key == "agent_skills"));
        assert!(keys.iter().any(|key| key == "agent_tools"));
        assert!(keys.iter().any(|key| key == "custom"));
    }

    #[test]
    fn test_app_doc_identity_is_required_and_validated() {
        // 缺 did 必须拒绝；旧 id 字段不再兼容。
        let missing_did = json!({
            "id": "did:bns:demo.tester",
            "doc_type": "app",
            "name": "demo",
            "version": "0.1.0",
            "owner": "did:bns:tester",
            "create_time": 1743008063u64,
            "last_update_time": 1743008063u64,
            "show_name": "Demo",
            "selector_type": "single",
            "pkg_list": {}
        });
        assert!(serde_json::from_value::<AppDoc>(missing_did).is_err());

        // 缺 doc_type 必须拒绝。
        let missing_doc_type = json!({
            "did": "did:bns:demo.tester",
            "name": "demo",
            "version": "0.1.0",
            "owner": "did:bns:tester",
            "create_time": 1743008063u64,
            "last_update_time": 1743008063u64,
            "show_name": "Demo",
            "selector_type": "single",
            "pkg_list": {}
        });
        assert!(serde_json::from_value::<AppDoc>(missing_doc_type).is_err());

        // doc_type 不是 app 必须拒绝。
        let wrong_doc_type = json!({
            "did": "did:bns:demo.tester",
            "doc_type": "zone",
            "name": "demo",
            "version": "0.1.0",
            "owner": "did:bns:tester",
            "create_time": 1743008063u64,
            "last_update_time": 1743008063u64,
            "show_name": "Demo",
            "selector_type": "single",
            "pkg_list": {}
        });
        assert!(serde_json::from_value::<AppDoc>(wrong_doc_type).is_err());
    }

    #[test]
    fn test_app_doc_builder_derives_frozen_bns_app_did() {
        let owner = DID::from_str("did:bns:tester").unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(pinned_package("all.web.demo-web.tester.bns.did#0.1.0"))
            .build()
            .unwrap();
        assert_eq!(doc.app_did(), &DID::new("bns", "demo-web.tester"));
        assert_eq!(doc.doc_type.to_string(), "app");

        // 显式 app_did 覆盖派生规则。
        let explicit = DID::from_str("did:bns:custom-app-name").unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .app_did(explicit.clone())
            .web_pkg(pinned_package("all.web.custom-app-name.bns.did#0.1.0"))
            .build()
            .unwrap();
        assert_eq!(doc.app_did(), &explicit);
    }

    #[test]
    fn test_app_doc_obj_id_uses_appdoc_type_and_version_fields_stay_separate() {
        let owner = DID::from_str("did:bns:tester").unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(pinned_package("all.web.demo-web.tester.bns.did#0.1.0"))
            .build()
            .unwrap();

        let (obj_id, canonical) = doc.gen_obj_id();
        assert_eq!(obj_id.obj_type, OBJ_TYPE_APP_DOC);
        // canonical body 内只有语义版本字段 version；document_version 是 resolver
        // metadata 的字段，绝不能出现在 App Document body 里。
        let value: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(value.get("version").and_then(|v| v.as_str()), Some("0.1.0"));
        assert!(value.get("document_version").is_none());
        assert_eq!(value.get("doc_type").and_then(|v| v.as_str()), Some("app"));
        assert_eq!(
            value.get("did").and_then(|v| v.as_str()),
            Some("did:bns:demo-web.tester")
        );
        assert!(value.get("id").is_none());
    }

    #[test]
    fn test_effective_selector_derivation_and_explicit_override() {
        // 已知 key 派生。
        let desc = SubPkgDesc::new("demo-img#0.1.0");
        let selector =
            SubPkgList::effective_selector("aarch64_docker_image", &desc).expect("derived");
        assert!(selector.matches_platform("linux", "arm64"));
        assert!(!selector.matches_platform("linux", "amd64"));

        // 平台无关 key 匹配一切。
        let web_selector = SubPkgList::effective_selector("web", &desc).expect("web derived");
        assert!(web_selector.matches_platform("windows", "x86_64"));

        // 未知 key 无显式 selector 不参与选择。
        assert!(SubPkgList::effective_selector("my_model", &desc).is_none());

        // 显式 selector 覆盖派生表。
        let mut desc = SubPkgDesc::new("demo-img#0.1.0");
        desc.selector = Some(PackageSelector::for_platform("windows", "amd64"));
        let selector = SubPkgList::effective_selector("aarch64_docker_image", &desc).unwrap();
        assert!(selector.matches_platform("windows", "x86_64"));
        assert!(!selector.matches_platform("linux", "aarch64"));

        // required 缺省为 true。
        assert!(desc.is_required());
        desc.required = Some(false);
        assert!(!desc.is_required());
    }

    #[test]
    fn test_app_doc_builder_agent_requires_agent_pkg() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let err = AppDoc::builder(
            AppType::Agent,
            "demo-agent-bad",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("Agent app must include `pkg_list.agent`"),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn test_app_doc_builder_agent_builds_with_agent_packages() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let doc = AppDoc::builder(
            AppType::Agent,
            "demo-agent",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .agent_pkg(pinned_package(
            "all.agent.demo-agent.example.com.bns.did#0.1.0",
        ))
        .agent_skills_pkg(pinned_package(
            "all.skills.demo-agent.example.com.bns.did#0.1.0",
        ))
        .build()
        .unwrap();

        assert_eq!(doc.get_app_type(), AppType::Agent);
        assert_eq!(
            doc.pkg_list.agent.as_ref().map(|pkg| pkg.pkg_id.as_str()),
            Some("all.agent.demo-agent.example.com.bns.did#0.1.0")
        );
        assert_eq!(
            doc.pkg_list
                .agent_skills
                .as_ref()
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("all.skills.demo-agent.example.com.bns.did#0.1.0")
        );
    }
}
