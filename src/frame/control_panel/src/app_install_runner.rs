//! 安装任务 runner：本地执行与恢复（P5.2）。
//!
//! beta2.2 起没有 TaskMgr runner inbox / task_ready 事件：安装 Task 只能由
//! control_panel 自己的已鉴权业务接口创建并直接执行。TaskManager 是唯一
//! 持久真相源；创建、确认和重试后立即本地执行，启动扫描和低频 sweep
//! 从持久状态恢复遗漏，不另建业务队列。

use crate::app_install_engine::InstallEngine;
use crate::app_install_engine::InstallTaskStatus;
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;

const SWEEP_INTERVAL_SECS: u64 = 60;

fn should_resume(status: InstallTaskStatus) -> bool {
    matches!(
        status,
        InstallTaskStatus::Pending | InstallTaskStatus::Running
    )
}

pub struct InstallRunner {
    engine: Arc<InstallEngine>,
}

impl InstallRunner {
    pub fn new(engine: Arc<InstallEngine>) -> Arc<Self> {
        Arc::new(Self { engine })
    }

    /// 启动恢复循环（服务启动时调用一次）。
    pub fn start(self: &Arc<Self>) {
        let runner = self.clone();
        tokio::spawn(async move {
            runner.startup_scan().await;
        });

        let runner = self.clone();
        tokio::spawn(async move {
            runner.sweep_loop().await;
        });
    }

    /// 业务入口已把 Task 持久化到 TaskManager；这里只启动同进程执行体。
    pub fn spawn_run(self: &Arc<Self>, task_id: String) {
        let runner = self.clone();
        tokio::spawn(async move {
            let _ = runner.engine.run_task(&task_id).await;
        });
    }

    /// 启动恢复：TaskManager 真相扫描，Pending/Running 恢复执行；
    /// WaitingForApproval 等确认、Paused 等 retry，都不动。
    async fn startup_scan(self: &Arc<Self>) {
        match self.engine.store().list_active().await {
            Ok(tasks) => {
                for task in tasks {
                    if should_resume(task.status) {
                        info!("recover install task {} ({:?})", task.id, task.status);
                        self.spawn_run(task.id.clone());
                    }
                }
            }
            Err(err) => warn!("startup install task scan failed: {err}"),
        }
    }

    /// 低频 sweep：修复一切遗漏（含 Running 态僵尸——进程内守卫保证不会
    /// 与在跑的执行体打架；真正跑着的任务 run_task 会因守卫直接返回）。
    /// 正常路径的低延迟由业务 RPC 直接启动执行体保证。
    async fn sweep_loop(self: &Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(SWEEP_INTERVAL_SECS)).await;
            match self.engine.store().list_active().await {
                Ok(tasks) => {
                    for task in tasks {
                        if should_resume(task.status) {
                            self.spawn_run(task.id.clone());
                        }
                    }
                }
                Err(err) => warn!("install sweep failed: {err}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_only_resumes_executable_states() {
        assert!(should_resume(InstallTaskStatus::Pending));
        assert!(should_resume(InstallTaskStatus::Running));
        assert!(!should_resume(InstallTaskStatus::WaitingForApproval));
        assert!(!should_resume(InstallTaskStatus::Paused));
        assert!(!should_resume(InstallTaskStatus::Completed));
        assert!(!should_resume(InstallTaskStatus::Failed));
        assert!(!should_resume(InstallTaskStatus::Canceled));
    }
}
