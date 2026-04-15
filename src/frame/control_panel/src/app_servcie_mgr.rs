use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, AppDoc, AppServiceSpec, AppType, ServiceInstallConfig, ServiceState,
    SystemConfigClient, SystemConfigError,
};
use log::warn;
use name_lib::DID;
use serde_json::{json, Value};
use std::sync::Arc;

/*
/configs/users/{$userid}/apps/{$appid}/spec -> AppServiceSpec

用户安装的标准App的分类
    - StaticWeb
    - ScriptHost(但还是跑在)
    - Agent
    - 标准AppService(Docker Image)

系统应用(跟着系统版本升级自动添加)
    - MessageHub
    - HomeStation
    - Content Store

应用信息的来源
    - AppSpec.AppDoc
    - 非结构化数据（Icon)统一得到ObjectId,然后通过 ndn/$objid 构造url
    - 构造 res/$appname/appicon.png 的url

桌面配置
    - 默认是在新窗口打开，还是用桌面窗口打开(AppSpec)


*/

// 系统内置应用清单：跟随系统版本升级，始终对所有用户可见
const SYSTEM_BUILTIN_APPS: &[&str] = &["messagehub", "homestation", "content-store"];
const SYSTEM_APP_AUTHOR: &str = "did:bns:buckyos";
const SYSTEM_APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl ControlPanelServer {
    // 构造系统内置应用的合成 AppServiceSpec。
    // 这些应用不存在于 system_config 的 users/{uid}/apps 路径下，而是跟系统版本一起发布，
    // 所以这里按 app_id 在代码里硬编码元信息并用 AppDoc::builder 动态构造。
    async fn get_system_app_spec(&self, app_id: &str) -> Result<AppServiceSpec, RPCErrors> {
        let (show_name, icon_url, description, app_index) = match app_id {
            "messagehub" => (
                "Message Hub",
                "res/messagehub/appicon.png",
                "BuckyOS 内置的统一消息中心",
                100u16,
            ),
            "homestation" => (
                "Home Station",
                "res/homestation/appicon.png",
                "BuckyOS 内置的家庭门户",
                101u16,
            ),
            "content-store" => (
                "Content Store",
                "res/content-store/appicon.png",
                "BuckyOS 内置的内容仓库",
                102u16,
            ),
            _ => {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown system app `{}`",
                    app_id
                )));
            }
        };

        let owner = DID::from_str(SYSTEM_APP_AUTHOR).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to build system owner DID: {}", err))
        })?;

        let app_doc = AppDoc::builder(
            AppType::Service,
            app_id,
            SYSTEM_APP_VERSION,
            SYSTEM_APP_AUTHOR,
            &owner,
        )
        .show_name(show_name)
        .app_icon_url(icon_url)
        .description_detail(description)
        .build()
        .map_err(|err| {
            RPCErrors::ReasonError(format!(
                "Failed to build system app doc `{}`: {}",
                app_id, err
            ))
        })?;

        Ok(AppServiceSpec {
            app_doc,
            app_index,
            user_id: String::new(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config: ServiceInstallConfig::default(),
        })
    }

    //输入用户名，返回用户可用应用服务列表（含系统默认应用，但不含系统服务)
    // 返回值包括 名字、描述、图标的ObjectId,类型,版本 以及其他的Meta信息
    pub(crate) async fn handle_apps_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let client = Self::app_service_system_config_client().await?;

        let mut apps: Vec<Value> = Vec::new();

        // 1) 用户已安装的 app / agent
        for base in ["apps", "agents"] {
            let is_agent = base == "agents";
            let base_key = format!("users/{}/{}", user_id, base);
            let app_ids = match client.list(&base_key).await {
                Ok(items) => items,
                Err(SystemConfigError::KeyNotFound(_)) => continue,
                Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
            };

            for app_id in app_ids {
                let spec_key = format!("{}/{}/spec", base_key, app_id);
                let record = match client.get(&spec_key).await {
                    Ok(record) => record,
                    Err(SystemConfigError::KeyNotFound(_)) => continue,
                    Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
                };

                match serde_json::from_str::<AppServiceSpec>(&record.value) {
                    Ok(spec) => {
                        apps.push(Self::build_app_summary(&spec, is_agent, false, &spec_key));
                    }
                    Err(error) => {
                        warn!(
                            "skip app `{}` for user `{}`: failed to parse spec `{}`: {}",
                            app_id, user_id, spec_key, error
                        );
                    }
                }
            }
        }

        // 2) 系统内置应用（MessageHub / HomeStation / Content Store 等）
        for system_app_id in SYSTEM_BUILTIN_APPS {
            match self.get_system_app_spec(system_app_id).await {
                Ok(mut spec) => {
                    spec.user_id = user_id.clone();
                    let spec_key = format!("system/apps/{}/spec", system_app_id);
                    apps.push(Self::build_app_summary(&spec, false, true, &spec_key));
                }
                Err(error) => {
                    warn!(
                        "skip built-in system app `{}`: {}",
                        system_app_id, error
                    );
                }
            }
        }

        apps.sort_by(|a, b| {
            let ai = a.get("app_index").and_then(|v| v.as_u64()).unwrap_or(0);
            let bi = b.get("app_index").and_then(|v| v.as_u64()).unwrap_or(0);
            ai.cmp(&bi)
        });

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "user_id": user_id,
                "total": apps.len(),
                "apps": apps,
            })),
            req.seq,
        ))
    }

    //获得一个app的全部详细信息，包括详细的安装配置
    pub(crate) async fn handle_app_detials(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let client = Self::app_service_system_config_client().await?;

        let candidates = [
            (false, format!("users/{}/apps/{}/spec", user_id, app_id)),
            (true, format!("users/{}/agents/{}/spec", user_id, app_id)),
        ];

        for (is_agent, spec_key) in candidates {
            let record = match client.get(&spec_key).await {
                Ok(record) => record,
                Err(SystemConfigError::KeyNotFound(_)) => continue,
                Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
            };

            let spec: AppServiceSpec =
                serde_json::from_str(&record.value).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to parse spec `{}`: {}",
                        spec_key, error
                    ))
                })?;
            let spec_value = serde_json::to_value(&spec).map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to serialize spec: {}", error))
            })?;
            let summary = Self::build_app_summary(&spec, is_agent, false, &spec_key);

            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "app_id": spec.app_id(),
                    "user_id": spec.user_id,
                    "is_agent": is_agent,
                    "is_system": false,
                    "spec_path": spec_key,
                    "summary": summary,
                    "spec": spec_value,
                })),
                req.seq,
            ));
        }

        // 用户 apps/agents 下找不到时，回退到系统内置应用
        if let Ok(mut spec) = self.get_system_app_spec(&app_id).await {
            spec.user_id = user_id.clone();
            let spec_key = format!("system/apps/{}/spec", app_id);
            let spec_value = serde_json::to_value(&spec).map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to serialize spec: {}", error))
            })?;
            let summary = Self::build_app_summary(&spec, false, true, &spec_key);

            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "app_id": spec.app_id(),
                    "user_id": spec.user_id,
                    "is_agent": false,
                    "is_system": true,
                    "spec_path": spec_key,
                    "summary": summary,
                    "spec": spec_value,
                })),
                req.seq,
            ));
        }

        Err(RPCErrors::ReasonError(format!(
            "App `{}` not found for user `{}`",
            app_id, user_id
        )))
    }

    async fn app_service_system_config_client() -> Result<Arc<SystemConfigClient>, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_system_config_client().await
    }

    // 将 AppServiceSpec 压扁成一份适合前端列表展示的摘要
    fn build_app_summary(
        spec: &AppServiceSpec,
        is_agent: bool,
        is_system: bool,
        spec_path: &str,
    ) -> Value {
        let app_type = spec.app_doc.get_app_type().to_string();
        let state = serde_json::to_value(&spec.state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        // 即使 app_icon_url 缺失，也给前端一个约定好的 res/$appname/appicon.png 回退地址
        let icon_res_url = format!("res/{}/appicon.png", spec.app_doc.name);

        json!({
            "app_id": spec.app_doc.name,
            "show_name": spec.app_doc.show_name,
            "version": spec.app_doc.version,
            "app_type": app_type,
            "app_icon_url": spec.app_doc.app_icon_url,
            "icon_res_url": icon_res_url,
            "author": spec.app_doc.author,
            "tags": spec.app_doc.tags,
            "categories": spec.app_doc.categories,
            "app_index": spec.app_index,
            "enable": spec.enable,
            "state": state,
            "expected_instance_count": spec.expected_instance_count,
            "is_agent": is_agent,
            "is_system": is_system,
            "spec_path": spec_path,
            "user_id": spec.user_id,
        })
    }
}
