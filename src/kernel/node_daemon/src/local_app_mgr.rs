use crate::app_loader::AppLoader;
use crate::run_item::*;
use async_trait::async_trait;
use buckyos_api::*;

//  目前本地app不支持docker
pub struct LocalAppRunItem {
    pub app_id: String,
    pub app_instance_config: LocalAppInstanceConfig,
    pub app_loader: AppLoader,
}

impl LocalAppRunItem {
    pub fn new(
        app_id: &String, // app_id@username@nodeid
        app_instance_config: LocalAppInstanceConfig,
    ) -> Self {
        LocalAppRunItem {
            app_id: app_id.clone(),
            app_instance_config: app_instance_config.clone(),
            app_loader: AppLoader::new_for_local(app_id.as_str(), app_instance_config),
        }
    }
}

#[async_trait]
impl RunItemControl for LocalAppRunItem {
    fn get_item_name(&self) -> Result<String> {
        //appid#userid
        let full_appid = format!("{}#{}", self.app_instance_config.user_id, self.app_id);
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
