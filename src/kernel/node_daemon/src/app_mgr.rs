use crate::app_loader::{process_node_session_id, AppLoader};
use crate::run_item::*;
use async_trait::async_trait;
use buckyos_api::AppServiceInstanceConfig;
use buckyos_api::*;
use buckyos_kit::buckyos_get_unix_timestamp;

// 统一交给 Rust AppLoader 处理 app runtime 的 deploy/start/stop/status。
pub struct AppRunItem {
    pub app_id: String,
    pub app_instance_config: AppServiceInstanceConfig,
    pub app_loader: AppLoader,
}

impl AppRunItem {
    pub fn new(
        app_instance_id: &AppInstanceId,
        app_service_config: AppServiceInstanceConfig,
    ) -> Self {
        let app_id = app_instance_id.app_id().to_string();
        AppRunItem {
            app_id: app_id,
            app_instance_config: app_service_config.clone(),
            app_loader: AppLoader::new_for_service(
                &app_instance_id.to_string(),
                app_service_config,
            ),
        }
    }

    async fn report_state(
        &self,
        state: ServiceInstanceState,
        health: DeploymentHealth,
        deployment_error: Option<DeploymentError>,
    ) {
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return;
        };
        let Some(device) = runtime.device_config.as_ref() else {
            return;
        };
        let Ok(client) = runtime.get_system_config_client().await else {
            return;
        };
        let now = buckyos_get_unix_timestamp();
        let report = ServiceInstanceReportInfo {
            node_id: self.app_instance_config.node_id.clone(),
            node_did: device.id.clone(),
            state,
            service_ports: self.app_instance_config.service_ports_config.clone(),
            last_update_time: now,
            start_time: 0,
            pid: 0,
            deployment: Some(self.app_instance_config.deployment.clone()),
            instance_epoch: format!(
                "node-daemon:{}:{}",
                std::process::id(),
                self.app_instance_config
                    .node_execution_spec
                    .app_instance_id
                    .runtime_key()
            ),
            node_session_id: process_node_session_id().to_string(),
            observed_at: now,
            expires_at: now.saturating_add(90),
            health,
            deployment_error,
        };
        let key = format!(
            "services/{}/instances/{}",
            self.app_instance_config.node_execution_spec.app_instance_id,
            self.app_instance_config.node_id
        );
        if let Ok(raw) = serde_json::to_string(&report) {
            let _ = client.set(&key, &raw).await;
        }
    }

    async fn report_deployment_failure(&self, code: &str, message: &str) {
        self.report_state(
            ServiceInstanceState::Exited,
            DeploymentHealth::Unhealthy,
            Some(DeploymentError {
                code: code.to_string(),
                message: message.to_string(),
                detail: None,
            }),
        )
        .await;
    }
}

#[async_trait]
impl RunItemControl for AppRunItem {
    fn get_item_name(&self) -> Result<String> {
        Ok(self
            .app_instance_config
            .node_execution_spec
            .app_instance_id
            .runtime_key())
    }

    fn get_item_kind(&self) -> &'static str {
        "app_service"
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        match self.app_loader.deploy().await {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                self.report_deployment_failure("deploy_failed", &message)
                    .await;
                Err(error)
            }
        }
    }

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        match self.app_loader.start().await {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                self.report_deployment_failure("start_failed", &message)
                    .await;
                Err(error)
            }
        }
    }

    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.app_loader.stop().await
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState> {
        let state = self.app_loader.status().await?;
        let health = self.app_loader.deployment_health().await?;
        self.report_state(state.clone(), health, None).await;
        Ok(state)
    }
}
