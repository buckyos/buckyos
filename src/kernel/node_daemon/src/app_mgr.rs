use crate::app_loader::AppLoader;
use crate::run_item::*;
use async_trait::async_trait;
use buckyos_api::AppServiceInstanceConfig;
use buckyos_api::*;

// 核心逻辑
// 非docker模式逻辑与标准的service item一致，但脚本调用是由app_loader来完成
// docker模式下
// 1. 通过app_loader的status脚本来判断是否存在（以镜像是否存在未标准）
// 2. 不存在，则要求app_loader安装镜像（可以指定media_info)
// 3. 由app_loader的start脚本来创建容器，创建的过程中可能会导入镜像
pub struct AppRunItem {
    pub app_id: String,
    pub app_instance_config: AppServiceInstanceConfig,
    pub app_loader: AppLoader,
}

impl AppRunItem {
    pub fn new(
        app_instance_id: &String, // app_id@username@nodeid
        app_service_config: AppServiceInstanceConfig,
    ) -> Self {
        let app_id = app_instance_id.split("@").nth(0).unwrap().to_string();
        AppRunItem {
            app_id: app_id,
            app_instance_config: app_service_config.clone(),
            app_loader: AppLoader::new_for_service(app_instance_id.as_str(), app_service_config),
        }
    }
}

#[async_trait]
impl RunItemControl for AppRunItem {
    fn get_item_name(&self) -> Result<String> {
        //appid#userid
        let full_appid = format!(
            "{}#{}",
            self.app_instance_config.app_spec.user_id, self.app_id
        );
        Ok(full_appid)
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.app_loader.deploy().await
    }

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.app_loader.start().await
    }

    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.app_loader.stop().await
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState> {
        self.app_loader.status().await
    }
}
