use ::kRPC::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Deref};
use crate::{PermissionRequest};
use ndn_lib::ObjId;
use package_lib::PackageMeta;
use name_lib::DID;

//buckyos 支持的应用类型,to_string后填写在app_doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppType {
    Service, // 系统服务 
    AppService, // 应用服务
    Web, //静态网页
    Agent, // AI Agent
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

impl ToString for AppType {
    fn to_string(&self) -> String {
        match self {
            AppType::Service => "service".to_string(),
            AppType::AppService => "dapp".to_string(),
            AppType::Web => "web".to_string(),
            AppType::Agent => "agent".to_string(),
        }
    }
}

//AppDoc \ InstallConfig \ ServiceSpec \ InstanceConfig 的基本设计
// App开发者发布的，有签名的Config是 AppDoc （已知应用，其更新应该走did-document的标准机制)
// AppDoc + InstallConfig后，保存在system_config（已安装应用）上的是 [AppServiceSpec],如果应用有更新，必要的时候是需要修改AppServiceSpec来执行更新的
// 调度器基于AppServiceSpec，部署在Node上的是 AppInstanceConfig (这个必然是自动构建的)
//    为了减少多次获取信息的一致性问题，AppInstanceConfig中包含了所有信息（包含AppDoc,InstallConfig)

#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct DataMountRecommend {
    pub mount_point: String,
    pub reason: HashMap<String,String>,//key: language_id, value: reason
}

#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct CustomServiceDesc {
    pub desc: HashMap<String,String>,//key: language_id, value: desc
}
//InstallConfigTips用来说明，该App有哪些在安装时需要配置的项目
#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct ServiceInstallConfigTips {

    //系统允许多个不同的app实现同一个服务，但有不同的“路由方法”
    //比如 如果系统里app1 有配置 {"smb":445},app2有配置 {"smb":445}，此时系统选择使用app2作为smb服务提供者，则最终按如下流程完成访问
    //   client->zone_gateway:445 --rtcp-> node_gateway:rtcp_stack -> docker_port 127:0.0.1:2190(调度器随机分配给app2) -> app2:445
    //                                                                docker_port 127.0.0.1:2189 -> app1:445
    //   此时基于app1.service_info可以通过 node_gateway:2189访问到app1的smb服务

    //service_name(like,web ,smb, dns, etc...) -> inner port
    
    #[serde(default)]
    pub service_ports: HashMap<String,u16>,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub custom_service_desc: HashMap<String,CustomServiceDesc>,//用于提示用户，该服务需要开放的端口最好是开放到哪个端口，并给出理由

    #[serde(default)]
    pub data_mount_point: Vec<String>,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub data_mount_recommend: HashMap<String,DataMountRecommend>,//用于提示用户，该服务需要挂载的数据目录最好是挂载到哪个目录，并给出理由
    #[serde(default)]
    pub local_cache_mount_point: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_param:Option<String>,
    #[serde(flatten)]
    pub custom_config:HashMap<String,serde_json::Value>,
} 

impl Default for ServiceInstallConfigTips {
    fn default() -> Self {
        Self {
            data_mount_point: vec![],
            data_mount_recommend: HashMap::new(),
            custom_service_desc: HashMap::new(),
            local_cache_mount_point: vec![],
            service_ports: HashMap::new(),
            container_param: None,
            start_param: None,
            custom_config: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct SubPkgDesc {
    pub pkg_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkg_objid:Option<ObjId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_name:Option<String>,//like buckyos/nightly-buckyos-filebrowser:0.4.1-amd64, 
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_digest:Option<String>,//docker digest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url:Option<String>,

}

#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct SubPkgList {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_docker_image: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_docker_image: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_win_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_win_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64_apple_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64_apple_app: Option<SubPkgDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<SubPkgDesc>,
    #[serde(flatten)]
    pub others: HashMap<String, SubPkgDesc>,
}

impl Default for SubPkgList {
    fn default() -> Self {
        Self {
            amd64_docker_image: None,
            aarch64_docker_image: None,
            amd64_win_app: None,
            aarch64_win_app: None,
            aarch64_apple_app: None,
            amd64_apple_app: None,
            web: None,
            others: HashMap::new(),
        }
    }
}

impl SubPkgList {

    pub fn get_app_pkg_id(&self) -> Option<String> {
        //根据编译时的目标系统，返回对应的app pkg_id
        if cfg!(target_os = "macos") {
            if let Some(pkg) = &self.aarch64_apple_app {
                return Some(pkg.pkg_id.clone());
            }
        } else if cfg!(target_os = "windows") {
            if let Some(pkg) = &self.amd64_win_app {
                return Some(pkg.pkg_id.clone());
            }
        }

        None
    }

    pub fn get_docker_image_pkg_id(&self) -> Option<String> {
        //根据当前编译期架构，返回对应的docker image pkg_id
        if cfg!(target_arch = "aarch64") {
            if let Some(pkg) = &self.aarch64_docker_image {
                return Some(pkg.pkg_id.clone());
            }
        } else {
            if let Some(pkg) = &self.amd64_docker_image {
                return Some(pkg.pkg_id.clone());
            }
        }

        None
    }
    pub fn get(&self, key: &str) -> Option<&SubPkgDesc> {
        match key {
            "amd64_docker_image" => self.amd64_docker_image.as_ref(),
            "aarch64_docker_image" => self.aarch64_docker_image.as_ref(),
            "amd64_win_app" => self.amd64_win_app.as_ref(),
            "aarch64_apple_app" => self.aarch64_apple_app.as_ref(),
            "web" => self.web.as_ref(),
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
        if let Some(pkg) = &self.amd64_win_app {
            list.push(("amd64_win_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_apple_app {
            list.push(("aarch64_apple_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.web {
            list.push(("web".to_string(), pkg));
        }
        for (k, v) in self.others.iter() {
            list.push((k.clone(), v));
        }
        list
    }
}

#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
#[serde(try_from = "String", into = "String")]
pub enum SelectorType {
    Single,
    Static,//no instance, only one static web page
    Random,
    ByEvent, //由特定的时间触发运行
    Custom(String),//custom selector type, like "round_robin"
}

impl Default for SelectorType {
    fn default() -> Self {
        Self::Single
    }
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


//App doc is store at Index-db, publish to bucky store
#[derive(Serialize, Deserialize,Clone, PartialEq, Debug)]
pub struct AppDoc {
    #[serde(flatten)]    
    pub _base: PackageMeta,
    pub show_name: String, // just for display, app_id is meta.pkg_name (like "buckyos-filebrowser")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_icon_url: Option<String>,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub req_capbilities: HashMap<String, i64>,//key: capability_name, value: required capability_value
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionRequest>,
    pub selector_type:SelectorType,
    //UI 应该根据install_config_tips的提示，来构造UI，得到最终的InstallConfig
    #[serde(default)]
    pub install_config_tips:ServiceInstallConfigTips,
    pub pkg_list: SubPkgList,
}



impl AppDoc {
    pub fn get_app_type(&self) -> AppType {
        if self.categories.len() > 0 {
            let mut result = AppType::Service;
            if let Ok(app_type) = AppType::try_from(self.categories[0].as_str()) {
                result = app_type;
            }
            result
        } else {
            AppType::Service
        }
    }

    pub fn from_pkg_meta(pkg_meta: &PackageMeta) -> Result<Self> {
        let pkg_json = serde_json::to_value(pkg_meta).unwrap();
        let result_self  = serde_json::from_value(pkg_json)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(result_self) 
    }

    pub fn to_pkg_meta(&self) -> Result<PackageMeta> {
        let pkg_json = serde_json::to_value(self).unwrap();
        let result_pkg_meta = serde_json::from_value(pkg_json)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(result_pkg_meta)
    }
}

impl Deref for AppDoc {
    type Target = PackageMeta;
    
    fn deref(&self) -> &Self::Target {
        &self._base
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
//   Agent: 暂时不支持


impl SubPkgDesc {
    pub fn new(pkg_id: impl Into<String>) -> Self {
        Self {
            pkg_id: pkg_id.into(),
            pkg_objid: None,
            docker_image_name: None,
            docker_image_digest: None,
            source_url: None,
        }
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
    meta: PackageMeta,
    show_name: Option<String>,
    app_icon_url: Option<String>,
    req_capbilities: HashMap<String, i64>,
    permissions: Vec<PermissionRequest>,
    selector_type: Option<SelectorType>,
    install_config_tips: ServiceInstallConfigTips,
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

        // IMPORTANT: must construct PackageMeta via its constructor so that
        // FileObject-related fields are initialized correctly.
        let mut meta = PackageMeta::new(name.as_str(), version.as_str(), author.as_str(), owner, None);

        // Best-effort fill optional fields commonly expected by AppDoc JSON.
        meta.size = 0;
        meta.exp = 0;
        meta.create_time = now;
        meta.last_update_time = now;
        meta.deps = HashMap::new();
        meta.categories = vec![app_type.to_string()];

        Self {
            app_type,
            meta,
            show_name: None,
            app_icon_url: None,
            req_capbilities: HashMap::new(),
            permissions: vec![],
            selector_type: None,
            install_config_tips: ServiceInstallConfigTips::default(),
            pkg_list: SubPkgList::default(),
            apply_default_permissions: true,
        }
    }

    pub fn show_name(mut self, show_name: impl Into<String>) -> Self {
        self.show_name = Some(show_name.into());
        self
    }

    pub fn app_icon_url(mut self, app_icon_url: impl Into<String>) -> Self {
        self.app_icon_url = Some(app_icon_url.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.meta.version_tag = Some(tag.into());
        self
    }

    pub fn exp(mut self, exp: u64) -> Self {
        self.meta.exp = exp;
        self
    }

    /// Set i18n description detail text.
    ///
    /// Stored into `PackageMeta.meta["description"]` in the following form:
    /// `{ "detail": { "<language_id>": "<text>" } }`.
    pub fn description(mut self, language_id: impl Into<String>, text: impl Into<String>) -> Self {
        let language_id = language_id.into();
        let text = text.into();

        let desc = self
            .meta
            .meta
            .entry("description".to_string())
            .or_insert_with(|| serde_json::json!({ "detail": {} }));

        match desc {
            serde_json::Value::Object(desc_obj) => {
                let detail = desc_obj
                    .entry("detail".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                match detail {
                    serde_json::Value::Object(detail_obj) => {
                        detail_obj.insert(language_id, serde_json::Value::String(text));
                    }
                    _ => {
                        // If existing "detail" is not an object, override it into an i18n map.
                        let mut detail_obj = serde_json::Map::new();
                        detail_obj.insert(language_id, serde_json::Value::String(text));
                        desc_obj.insert("detail".to_string(), serde_json::Value::Object(detail_obj));
                    }
                }
            }
            _ => {
                // If existing "description" is not an object, override it into an i18n structure.
                self.meta.meta.insert(
                    "description".to_string(),
                    serde_json::json!({ "detail": { language_id: text } }),
                );
            }
        }
        self
    }

    /// Advanced: set raw `PackageMeta.meta["description"]` value directly.
    pub fn description_raw(mut self, description: serde_json::Value) -> Self {
        self.meta
            .meta
            .insert("description".to_string(), description);
        self
    }

    pub fn description_detail(self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        // Backward-compatible helper: write a single-language "en" description.
        self.description("en", detail)
    }

    pub fn add_dep(mut self, pkg_name: impl Into<String>, version_req: impl Into<String>) -> Self {
        self.meta.deps.insert(pkg_name.into(), version_req.into());
        self
    }

    pub fn selector_type(mut self, selector_type: SelectorType) -> Self {
        self.selector_type = Some(selector_type);
        self
    }

    pub fn req_capability(mut self, name: impl Into<String>, value: i64) -> Self {
        self.req_capbilities.insert(name.into(), value);
        self
    }

    pub fn add_permission(mut self, permission: PermissionRequest) -> Self {
        self.permissions.push(permission);
        self
    }

    pub fn apply_default_permissions(mut self, apply: bool) -> Self {
        self.apply_default_permissions = apply;
        self
    }

    // -------- install tips helpers --------
    pub fn add_data_mount_point(mut self, mount_point: impl Into<String>) -> Self {
        self.install_config_tips.data_mount_point.push(mount_point.into());
        self
    }

    pub fn add_local_cache_mount_point(mut self, mount_point: impl Into<String>) -> Self {
        self.install_config_tips
            .local_cache_mount_point
            .push(mount_point.into());
        self
    }

    pub fn service_port(mut self, service_name: impl Into<String>, port: u16) -> Self {
        self.install_config_tips
            .service_ports
            .insert(service_name.into(), port);
        self
    }

    pub fn container_param(mut self, param: impl Into<String>) -> Self {
        self.install_config_tips.container_param = Some(param.into());
        self
    }

    pub fn start_param(mut self, param: impl Into<String>) -> Self {
        self.install_config_tips.start_param = Some(param.into());
        self
    }

    pub fn install_custom(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.install_config_tips.custom_config.insert(key.into(), value);
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

    pub fn web_pkg(mut self, desc: SubPkgDesc) -> Self {
        self.pkg_list.web = Some(desc);
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

        use crate::GrantMode;
        self.permissions.push(PermissionRequest {
            scope: "fs.data".to_string(),
            grant: GrantMode::All,
            required: true,
            actions: vec!["read".to_string(), "write".to_string()],
            items: vec![],
            constraints: None,
        });
        self.permissions.push(PermissionRequest {
            scope: "fs.cache".to_string(),
            grant: GrantMode::All,
            required: true,
            actions: vec!["read".to_string(), "write".to_string()],
            items: vec![],
            constraints: None,
        });
        self.permissions.push(PermissionRequest {
            scope: "fs.library".to_string(),
            grant: GrantMode::All,
            required: false,
            actions: vec!["read".to_string()],
            items: vec![],
            constraints: None,
        });
    }

    pub fn build(mut self) -> Result<AppDoc> {
        if self.app_type == AppType::Agent {
            return Err(RPCErrors::ReasonError(
                "AppType::Agent is not supported yet".to_string(),
            ));
        }

        let has_docker = self.pkg_list.amd64_docker_image.is_some()
            || self.pkg_list.aarch64_docker_image.is_some();
        let has_web = self.pkg_list.web.is_some();
        let has_native_app = self.pkg_list.amd64_win_app.is_some()
            || self.pkg_list.aarch64_win_app.is_some()
            || self.pkg_list.amd64_apple_app.is_some()
            || self.pkg_list.aarch64_apple_app.is_some();

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
            }
            AppType::AppService => {
                if !has_docker {
                    errors.push("AppService app must include docker images (set `amd64_docker_image` and/or `aarch64_docker_image`)".to_string());
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

                // Web is always static and should not request permissions.
                self.selector_type = Some(SelectorType::Static);
                self.permissions.clear();
                self.install_config_tips = ServiceInstallConfigTips::default();
            }
            AppType::Agent => unreachable!(),
        }

        if !errors.is_empty() {
            return Err(RPCErrors::ReasonError(errors.join("; ")));
        }

        // Provide sane defaults for human-readable fields.
        let show_name = self
            .show_name
            .clone()
            .or_else(|| Some(self.meta.name.clone()))
            .unwrap_or_else(|| "Unnamed App".to_string());

        if !self.meta.meta.contains_key("description") {
            // Default i18n description.
            self.meta.meta.insert(
                "description".to_string(),
                serde_json::json!({ "detail": { "en": show_name.clone() } }),
            );
        }

        Ok(AppDoc {
            _base: self.meta,
            show_name,
            app_icon_url: self.app_icon_url,
            req_capbilities: self.req_capbilities,
            permissions: self.permissions,
            selector_type: self.selector_type.unwrap_or_default(),
            install_config_tips: self.install_config_tips,
            pkg_list: self.pkg_list,
        })
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
    #[tokio::test]
    async fn test_get_parse_app_doc() {
        let app_doc = json!({
            "name": "buckyos_filebrowser",
            "version": "0.4.1",
            "tag": "latest",
            "size":0,
            "show_name": "BuckyOS File Browser",
            "description": {
                "detail": "BuckyOS File Browser"
            },
            "author": "did:web:buckyos.ai",
            "owner": "did:web:buckyos.ai",
            "create_time": 1743008063u64,
            "last_update_time": 1743008063u64,
            "exp": 1837616063u64,
            "selector_type": "single",
            "install_config_tips": {
                "data_mount_point": ["/srv/", "/database/", "/config/"],
                "local_cache_mount_point": [],
                "service_ports": {
                    "www": 80
                }
            },
            "pkg_list": {
                "amd64_docker_image": {
                    "pkg_id": "nightly-linux-amd64.buckyos_filebrowser-img#0.4.1",
                    "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-amd64"
                },
                "aarch64_docker_image": {
                    "pkg_id": "nightly-linux-aarch64.buckyos_filebrowser-img#0.4.1",
                    "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-aarch64"
                },
                "amd64_win_app": {
                    "pkg_id": "nightly-windows-amd64.buckyos_filebrowser-bin#0.4.1"
                },
                "aarch64_apple_app": {
                    "pkg_id": "nightly-apple-aarch64.buckyos_filebrowser-bin#0.4.1"
                },
                "amd64_apple_app": {
                    "pkg_id": "nightly-apple-amd64.buckyos_filebrowser-bin#0.4.1"
                }
            },
            "deps": {
                "nightly-linux-amd64.buckyos_filebrowser-img": "0.4.1",
                "nightly-linux-aarch64.buckyos_filebrowser-img": "0.4.1",
                "nightly-windows-amd64.buckyos_filebrowser-bin": "0.4.1",
                "nightly-apple-amd64.buckyos_filebrowser-bin": "0.4.1",
                "nightly-apple-aarch64.buckyos_filebrowser-bin": "0.4.1"
            }
        });
        let app_doc:AppDoc = serde_json::from_value(app_doc).unwrap();
        println!("{}#{}", app_doc.name, app_doc.version);
        let app_doc_str = serde_json::to_string_pretty(&app_doc).unwrap();
        println!("{}", app_doc_str);

        let pkg_meta = app_doc.to_pkg_meta().unwrap();
        println!("{}", serde_json::to_string_pretty(&pkg_meta).unwrap());
        let app_doc_from_pkg_meta = AppDoc::from_pkg_meta(&pkg_meta).unwrap();
        println!("{}", serde_json::to_string_pretty(&app_doc_from_pkg_meta).unwrap());

        assert_eq!(app_doc, app_doc_from_pkg_meta);
    }

    #[test]
    fn test_app_doc_builder_web_enforces_static_and_no_permissions() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let doc = AppDoc::builder(
            AppType::Web,
            "demo_web",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .show_name("Demo Web")
        .description("en", "Demo Web Description")
        .description("zh", "演示网页应用描述")
        .web_pkg(SubPkgDesc::new("demo_web-web#0.1.0"))
        .add_permission(PermissionRequest {
            scope: "fs.data".to_string(),
            grant: crate::GrantMode::All,
            required: true,
            actions: vec!["read".to_string()],
            items: vec![],
            constraints: None,
        })
        .build()
        .unwrap();

        println!(
            "built web app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );

        assert_eq!(doc.selector_type, SelectorType::Static);
        assert!(doc.permissions.is_empty());

        let sys_testdoc = r#"
{
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
    "install_config_tips": {
    },
    "pkg_list": {
      "web": {
        "pkg_id": "nightly-linux-amd64.buckyos_systest#0.5.1"
      }
    }
  }     
"#;
        let parsed_doc: AppDoc = serde_json::from_str(sys_testdoc).unwrap();
        assert_eq!(parsed_doc.selector_type, SelectorType::Static);
        
        
    }

    #[test]
    fn test_app_doc_builder_service_minimal_ok_and_rejects_docker_web() {
        let owner = DID::from_str("did:web:example.com").unwrap();

        // Minimal service should build successfully (no docker, no web).
        let doc = AppDoc::builder(
            AppType::Service,
            "demo_service",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .show_name("Demo Service")
        .build()
        .unwrap();
        println!(
            "built service app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );
        assert_eq!(doc.get_app_type(), AppType::Service);

        // Service must reject docker image.
        let err = AppDoc::builder(
            AppType::Service,
            "demo_service_bad",
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
            "demo_service_bad2",
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
    fn test_app_doc_builder_appservice_requires_docker_and_sets_default_permissions() {
        let owner = DID::from_str("did:web:example.com").unwrap();

        // AppService must require docker image.
        let err = AppDoc::builder(
            AppType::AppService,
            "demo_dapp_bad",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("must include docker images"),
            "unexpected error: {:?}",
            err
        );

        // AppService should build with docker and auto-fill default permissions when not provided.
        let doc = AppDoc::builder(
            AppType::AppService,
            "demo_dapp",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .amd64_docker_image(
            SubPkgDesc::new("demo_dapp-img#0.1.0").docker_image_name("buckyos/demo_dapp:0.1.0"),
        )
        .build()
        .unwrap();

        println!(
            "built appservice app_doc:\n{}",
            serde_json::to_string_pretty(&doc).unwrap()
        );
        assert_eq!(doc.get_app_type(), AppType::AppService);

        let scopes: Vec<&str> = doc.permissions.iter().map(|p| p.scope.as_str()).collect();
        assert!(scopes.contains(&"fs.data"), "permissions: {:?}", doc.permissions);
        assert!(scopes.contains(&"fs.cache"), "permissions: {:?}", doc.permissions);
        assert!(scopes.contains(&"fs.library"), "permissions: {:?}", doc.permissions);
    }

    #[test]
    fn test_app_doc_builder_web_requires_web_pkg() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let err = AppDoc::builder(
            AppType::Web,
            "demo_web_bad",
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
    fn test_app_doc_builder_agent_not_supported() {
        let owner = DID::from_str("did:web:example.com").unwrap();
        let err = AppDoc::builder(
            AppType::Agent,
            "demo_agent",
            "0.1.0",
            "did:web:example.com",
            &owner,
        )
        .build()
        .err()
        .unwrap();
        assert!(
            format!("{:?}", err).contains("not supported"),
            "unexpected error: {:?}",
            err
        );
    }
}
