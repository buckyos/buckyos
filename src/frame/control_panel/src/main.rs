use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_api::{CONTROL_PANEL_SERVICE_PORT, SystemConfigClient};
use buckyos_kit::*;
use bytes::Bytes;
use cyfs_gateway_lib::*;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use serde_json::*;
use server_runner::*;
use std::sync::Arc;
use std::{net::IpAddr, time::Duration};
use sysinfo::{DiskRefreshKind, Disks, System};

// RPC docs live under doc/dashboard. UI endpoints use "ui.*" as canonical names;
// "main/layout/dashboard" are kept as legacy aliases.

fn bytes_to_gb(bytes: u64) -> f64 {
    (bytes as f64) / 1024.0 / 1024.0 / 1024.0
}

#[derive(Clone)]
struct ControlPanelServer {}

impl ControlPanelServer {
    pub fn new() -> Self {
        ControlPanelServer {}
    }

    async fn handle_main(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "test":"test",
            })),
            req.id,
        ))
    }

    async fn handle_layout(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let layout = json!({
            "profile": {
                "name": "Admin User",
                "email": "admin@buckyos.io",
                "avatar": "https://i.pravatar.cc/64?img=12"
            },
            "systemStatus": {
                "label": "System Online",
                "state": "online",
                "networkPeers": 10,
                "activeSessions": 23
            }
        });

        Ok(RPCResponse::new(RPCResult::Success(layout), req.id))
    }

    async fn handle_dashboard(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_memory();
        system.refresh_cpu_usage();
        // Wait a moment so CPU usage has a meaningful delta before the second refresh.
        tokio::time::sleep(Duration::from_millis(200)).await;
        system.refresh_cpu_usage();

        let cpu_usage = system.global_cpu_usage() as f64;
        let cpu_brand = system
            .cpus()
            .get(0)
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let cpu_cores = system.cpus().len() as u64;
        let total_memory_bytes = system.total_memory();
        let used_memory_bytes = system.used_memory();
        let memory_percent = if total_memory_bytes > 0 {
            ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let mut storage_slices: Vec<Value> = Vec::new();
        let mut disks_detail: Vec<Value> = Vec::new();
        let mut storage_capacity_bytes: u64 = 0;
        let mut storage_used_bytes: u64 = 0;
        let palette = [
            "#1d4ed8", "#6b7280", "#22c55e", "#facc15", "#38bdf8", "#a855f7",
        ];

        let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything());
        disks.refresh(true);

        for (idx, disk) in disks.list().iter().enumerate() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            storage_capacity_bytes = storage_capacity_bytes.saturating_add(total);
            storage_used_bytes = storage_used_bytes.saturating_add(used);

            let used_percent = if total > 0 {
                ((used as f64 / total as f64) * 100.0).round()
            } else {
                0.0
            };

            storage_slices.push(json!({
                "label": disk.name().to_string_lossy(),
                "value": used_percent,
                "color": palette[idx % palette.len()],
            }));

            disks_detail.push(json!({
                "label": disk.name().to_string_lossy(),
                "totalGb": bytes_to_gb(total),
                "usedGb": bytes_to_gb(used),
                "fs": disk.file_system().to_string_lossy(),
                "mount": disk.mount_point().to_string_lossy(),
            }));
        }

        if storage_slices.is_empty() {
            storage_slices.push(json!({
                "label": "Storage",
                "value": 0,
                "color": "#6b7280",
            }));
        }

        let storage_capacity_gb = bytes_to_gb(storage_capacity_bytes);
        let storage_used_gb = bytes_to_gb(storage_used_bytes);
        let memory_total_gb = bytes_to_gb(total_memory_bytes);
        let memory_used_gb = bytes_to_gb(used_memory_bytes);

        let device_name = System::host_name().unwrap_or_else(|| "Local Node".to_string());
        let device_info = json!({
            "name": device_name,
            "role": "server",
            "status": "online",
            "uptimeHours": System::uptime() / 3600,
            "cpu": (cpu_usage.round() as u64).min(100),
            "memory": memory_percent as u64,
        });

        let base_cpu = cpu_usage.round() as i64;
        let timeline: Vec<Value> = (0..6)
            .map(|step| {
                let cpu_val = (base_cpu + step as i64 * 2 - 5).clamp(0, 100) as u64;
                json!({
                    "time": format!("{:02}:{:02}", (step * 5) / 60, (step * 5) % 60),
                    "cpu": cpu_val,
                    "memory": memory_percent as u64,
                })
            })
            .collect();

        let dashboard = json!({
            "recentEvents": [
                { "title": "System backup completed", "subtitle": "2 mins ago", "tone": "success" },
                { "title": "High memory usage detected", "subtitle": "15 mins ago", "tone": "warning" },
                { "title": "New device connected: iPhone 15", "subtitle": "1 hour ago", "tone": "info" },
                { "title": "dApp \"FileSync\" updated successfully", "subtitle": "2 hours ago", "tone": "success" },
                { "title": "New admin policy applied", "subtitle": "Yesterday", "tone": "info" }
            ],
            "dapps": [
                { "name": "FileSync", "icon": "🗂️", "status": "running" },
                { "name": "SecureChat", "icon": "💬", "status": "stopped" },
                { "name": "CloudBridge", "icon": "🌉", "status": "stopped" },
                { "name": "PhotoVault", "icon": "📷", "status": "running" },
                { "name": "DataAnalyzer", "icon": "📊", "status": "running" },
                { "name": "WebPortal", "icon": "🌐", "status": "running" }
            ],
            "resourceTimeline": timeline,
            "storageSlices": storage_slices,
            "storageCapacityGb": storage_capacity_gb,
            "storageUsedGb": storage_used_gb,
            "devices": [device_info],
            "memory": {
                "totalGb": memory_total_gb,
                "usedGb": memory_used_gb,
                "usagePercent": memory_percent,
            },
            "cpu": {
                "usagePercent": cpu_usage,
                "model": cpu_brand,
                "cores": cpu_cores,
            },
            "disks": disks_detail
        });

        Ok(RPCResponse::new(RPCResult::Success(dashboard), req.id))
    }

    async fn handle_system_config_test(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = req
            .params
            .get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("boot/config")
            .to_string();
        let service_url = req
            .params
            .get("service_url")
            .and_then(|value| value.as_str());
        let session_token = req.token.clone().or_else(|| {
            req.params
                .get("session_token")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        });

        let client = SystemConfigClient::new(service_url, session_token.as_deref());
        let value = client
            .get(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "value": value.value,
                "version": value.version,
                "isChanged": value.is_changed,
            })),
            req.id,
        ))
    }

    async fn handle_unimplemented(
        &self,
        req: RPCRequest,
        purpose: &'static str,
    ) -> Result<RPCResponse, RPCErrors> {
        Err(RPCErrors::ReasonError(format!(
            "Not implemented: {} ({})",
            req.method, purpose
        )))
    }
}

#[async_trait]
impl RPCHandler for ControlPanelServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            // Core / UI bootstrap
            "main" | "ui.main" => self.handle_main(req).await,
            "layout" | "ui.layout" => self.handle_layout(req).await,
            "dashboard" | "ui.dashboard" => self.handle_dashboard(req).await,
            // Auth
            "auth.login" => self
                .handle_unimplemented(req, "Authenticate admin/user session")
                .await,
            "auth.logout" => self.handle_unimplemented(req, "Terminate session").await,
            "auth.refresh" => self
                .handle_unimplemented(req, "Refresh token/session")
                .await,
            // User & Role
            "user.list" => self.handle_unimplemented(req, "List users").await,
            "user.get" => self.handle_unimplemented(req, "Get user detail").await,
            "user.create" => self.handle_unimplemented(req, "Create user").await,
            "user.update" => self.handle_unimplemented(req, "Update user").await,
            "user.delete" => self.handle_unimplemented(req, "Delete user").await,
            "user.role.list" => self.handle_unimplemented(req, "List roles/policies").await,
            "user.role.update" => self.handle_unimplemented(req, "Update role/policy").await,
            // System
            "system.overview" => self.handle_unimplemented(req, "System overview").await,
            "system.status" => self.handle_unimplemented(req, "Health status").await,
            "system.metrics" => self.handle_unimplemented(req, "CPU/mem/net/disk").await,
            "system.update.check" => self.handle_unimplemented(req, "Check updates").await,
            "system.update.apply" => self.handle_unimplemented(req, "Apply update").await,
            "system.config.test" => self.handle_system_config_test(req).await,
            // Storage
            "storage.volumes" => self.handle_unimplemented(req, "List volumes/arrays").await,
            "storage.volume.get" => self.handle_unimplemented(req, "Get volume detail").await,
            "storage.volume.create" => self.handle_unimplemented(req, "Create volume").await,
            "storage.volume.expand" => self.handle_unimplemented(req, "Expand volume").await,
            "storage.volume.delete" => self.handle_unimplemented(req, "Delete volume").await,
            "storage.disks" => self.handle_unimplemented(req, "List physical disks").await,
            "storage.smart" => self.handle_unimplemented(req, "Disk SMART info").await,
            "storage.raid.rebuild" => self.handle_unimplemented(req, "RAID rebuild").await,
            // Shares
            "share.list" => self.handle_unimplemented(req, "List shared folders").await,
            "share.get" => self.handle_unimplemented(req, "Get share detail").await,
            "share.create" => self.handle_unimplemented(req, "Create share").await,
            "share.update" => self.handle_unimplemented(req, "Update share").await,
            "share.delete" => self.handle_unimplemented(req, "Delete share").await,
            // Files
            "files.browse" => self.handle_unimplemented(req, "List directory entries").await,
            "files.stat" => self.handle_unimplemented(req, "File metadata").await,
            "files.mkdir" => self.handle_unimplemented(req, "Create folder").await,
            "files.delete" => self.handle_unimplemented(req, "Delete file/folder").await,
            "files.move" => self.handle_unimplemented(req, "Move/rename").await,
            "files.copy" => self.handle_unimplemented(req, "Copy").await,
            "files.upload.init" => self
                .handle_unimplemented(req, "Init multipart upload")
                .await,
            "files.upload.part" => self.handle_unimplemented(req, "Upload part").await,
            "files.upload.complete" => self
                .handle_unimplemented(req, "Complete upload")
                .await,
            "files.download" => self.handle_unimplemented(req, "Download file").await,
            // Backup
            "backup.jobs" => self.handle_unimplemented(req, "List backup jobs").await,
            "backup.job.create" => self.handle_unimplemented(req, "Create backup job").await,
            "backup.job.run" => self.handle_unimplemented(req, "Run backup job").await,
            "backup.job.stop" => self.handle_unimplemented(req, "Stop backup job").await,
            "backup.targets" => self.handle_unimplemented(req, "List backup targets").await,
            "backup.restore" => self.handle_unimplemented(req, "Restore backup").await,
            // Apps
            "apps.list" => self.handle_unimplemented(req, "List installed apps").await,
            "apps.install" => self.handle_unimplemented(req, "Install app").await,
            "apps.update" => self.handle_unimplemented(req, "Update app").await,
            "apps.uninstall" => self.handle_unimplemented(req, "Uninstall app").await,
            "apps.start" => self.handle_unimplemented(req, "Start app").await,
            "apps.stop" => self.handle_unimplemented(req, "Stop app").await,
            // Network
            "network.interfaces" => self.handle_unimplemented(req, "List interfaces").await,
            "network.interface.update" => self
                .handle_unimplemented(req, "Update interface config")
                .await,
            "network.dns" => self.handle_unimplemented(req, "Get/set DNS").await,
            "network.ddns" => self.handle_unimplemented(req, "Get/set DDNS").await,
            "network.firewall.rules" => self
                .handle_unimplemented(req, "List firewall rules")
                .await,
            "network.firewall.update" => self
                .handle_unimplemented(req, "Update firewall rules")
                .await,
            // Device
            "device.list" => self.handle_unimplemented(req, "List devices/clients").await,
            "device.block" => self.handle_unimplemented(req, "Block device").await,
            "device.unblock" => self.handle_unimplemented(req, "Unblock device").await,
            // Notification
            "notification.list" => self.handle_unimplemented(req, "List notifications/events").await,
            // Logs
            "log.system" => self.handle_unimplemented(req, "System logs").await,
            "log.access" => self.handle_unimplemented(req, "Access logs").await,
            // Security
            "security.2fa.enable" => self.handle_unimplemented(req, "Enable 2FA").await,
            "security.2fa.disable" => self.handle_unimplemented(req, "Disable 2FA").await,
            "security.keys" => self.handle_unimplemented(req, "List/revoke API keys").await,
            // File Services (SMB/NFS/FTP/WebDAV/...)
            "file_service.list" => self.handle_unimplemented(req, "List file services").await,
            "file_service.smb.get" => self.handle_unimplemented(req, "Get SMB config").await,
            "file_service.smb.update" => self.handle_unimplemented(req, "Update SMB config").await,
            "file_service.nfs.get" => self.handle_unimplemented(req, "Get NFS config").await,
            "file_service.nfs.update" => self.handle_unimplemented(req, "Update NFS config").await,
            "file_service.afp.get" => self.handle_unimplemented(req, "Get AFP config").await,
            "file_service.afp.update" => self.handle_unimplemented(req, "Update AFP config").await,
            "file_service.ftp.get" => self.handle_unimplemented(req, "Get FTP config").await,
            "file_service.ftp.update" => self.handle_unimplemented(req, "Update FTP config").await,
            "file_service.webdav.get" => self.handle_unimplemented(req, "Get WebDAV config").await,
            "file_service.webdav.update" => self.handle_unimplemented(req, "Update WebDAV config").await,
            "file_service.rsync.get" => self.handle_unimplemented(req, "Get rsync config").await,
            "file_service.rsync.update" => self.handle_unimplemented(req, "Update rsync config").await,
            "file_service.sftp.get" => self.handle_unimplemented(req, "Get SFTP config").await,
            "file_service.sftp.update" => self.handle_unimplemented(req, "Update SFTP config").await,
            "file_service.ssh.get" => self.handle_unimplemented(req, "Get SSH config").await,
            "file_service.ssh.update" => self.handle_unimplemented(req, "Update SSH config").await,
            // iSCSI
            "iscsi.targets" => self.handle_unimplemented(req, "List iSCSI targets").await,
            "iscsi.target.create" => self.handle_unimplemented(req, "Create iSCSI target").await,
            "iscsi.target.update" => self.handle_unimplemented(req, "Update iSCSI target").await,
            "iscsi.target.delete" => self.handle_unimplemented(req, "Delete iSCSI target").await,
            "iscsi.luns" => self.handle_unimplemented(req, "List iSCSI LUNs").await,
            "iscsi.lun.create" => self.handle_unimplemented(req, "Create iSCSI LUN").await,
            "iscsi.lun.update" => self.handle_unimplemented(req, "Update iSCSI LUN").await,
            "iscsi.lun.delete" => self.handle_unimplemented(req, "Delete iSCSI LUN").await,
            "iscsi.sessions" => self.handle_unimplemented(req, "List iSCSI sessions").await,
            // Snapshot
            "snapshot.list" => self.handle_unimplemented(req, "List snapshots").await,
            "snapshot.create" => self.handle_unimplemented(req, "Create snapshot").await,
            "snapshot.delete" => self.handle_unimplemented(req, "Delete snapshot").await,
            "snapshot.restore" => self.handle_unimplemented(req, "Restore snapshot").await,
            "snapshot.schedule.list" => self.handle_unimplemented(req, "List snapshot schedules").await,
            "snapshot.schedule.update" => self.handle_unimplemented(req, "Update snapshot schedule").await,
            // Replication
            "replication.jobs" => self.handle_unimplemented(req, "List replication jobs").await,
            "replication.job.create" => self.handle_unimplemented(req, "Create replication job").await,
            "replication.job.run" => self.handle_unimplemented(req, "Run replication job").await,
            "replication.job.pause" => self.handle_unimplemented(req, "Pause replication job").await,
            "replication.job.delete" => self.handle_unimplemented(req, "Delete replication job").await,
            "replication.status" => self.handle_unimplemented(req, "Replication status").await,
            // Sync
            "sync.providers" => self.handle_unimplemented(req, "List sync providers").await,
            "sync.tasks" => self.handle_unimplemented(req, "List sync tasks").await,
            "sync.task.create" => self.handle_unimplemented(req, "Create sync task").await,
            "sync.task.run" => self.handle_unimplemented(req, "Run sync task").await,
            "sync.task.pause" => self.handle_unimplemented(req, "Pause sync task").await,
            "sync.task.resume" => self.handle_unimplemented(req, "Resume sync task").await,
            "sync.task.delete" => self.handle_unimplemented(req, "Delete sync task").await,
            // Quota
            "quota.get" => self.handle_unimplemented(req, "Get quotas").await,
            "quota.update" => self.handle_unimplemented(req, "Update quota").await,
            "quota.defaults" => self.handle_unimplemented(req, "Get quota defaults").await,
            // ACL / Permissions
            "acl.get" => self.handle_unimplemented(req, "Get ACL").await,
            "acl.update" => self.handle_unimplemented(req, "Update ACL").await,
            "acl.reset" => self.handle_unimplemented(req, "Reset ACL").await,
            // Recycle Bin
            "recycle_bin.get" => self.handle_unimplemented(req, "Get recycle bin settings").await,
            "recycle_bin.update" => self.handle_unimplemented(req, "Update recycle bin settings").await,
            "recycle_bin.list" => self.handle_unimplemented(req, "List recycled items").await,
            "recycle_bin.restore" => self.handle_unimplemented(req, "Restore recycled item").await,
            "recycle_bin.delete" => self.handle_unimplemented(req, "Delete recycled item").await,
            // Index / Search
            "index.status" => self.handle_unimplemented(req, "Index status").await,
            "index.rebuild" => self.handle_unimplemented(req, "Rebuild index").await,
            "search.query" => self.handle_unimplemented(req, "Search query").await,
            // Media
            "media.library.scan" => self.handle_unimplemented(req, "Scan media library").await,
            "media.library.status" => self.handle_unimplemented(req, "Media library status").await,
            "media.dlna.get" => self.handle_unimplemented(req, "Get DLNA config").await,
            "media.dlna.update" => self.handle_unimplemented(req, "Update DLNA config").await,
            // Download
            "download.tasks" => self.handle_unimplemented(req, "List download tasks").await,
            "download.task.create" => self.handle_unimplemented(req, "Create download task").await,
            "download.task.pause" => self.handle_unimplemented(req, "Pause download task").await,
            "download.task.resume" => self.handle_unimplemented(req, "Resume download task").await,
            "download.task.delete" => self.handle_unimplemented(req, "Delete download task").await,
            // Container
            "container.list" => self.handle_unimplemented(req, "List containers").await,
            "container.create" => self.handle_unimplemented(req, "Create container").await,
            "container.start" => self.handle_unimplemented(req, "Start container").await,
            "container.stop" => self.handle_unimplemented(req, "Stop container").await,
            "container.update" => self.handle_unimplemented(req, "Update container").await,
            "container.delete" => self.handle_unimplemented(req, "Delete container").await,
            "container.images" => self.handle_unimplemented(req, "List container images").await,
            "container.image.pull" => self.handle_unimplemented(req, "Pull container image").await,
            "container.image.remove" => self.handle_unimplemented(req, "Remove container image").await,
            // VM
            "vm.list" => self.handle_unimplemented(req, "List VMs").await,
            "vm.create" => self.handle_unimplemented(req, "Create VM").await,
            "vm.start" => self.handle_unimplemented(req, "Start VM").await,
            "vm.stop" => self.handle_unimplemented(req, "Stop VM").await,
            "vm.delete" => self.handle_unimplemented(req, "Delete VM").await,
            "vm.snapshot.create" => self.handle_unimplemented(req, "Create VM snapshot").await,
            "vm.snapshot.restore" => self.handle_unimplemented(req, "Restore VM snapshot").await,
            // Certificate
            "cert.list" => self.handle_unimplemented(req, "List certificates").await,
            "cert.issue" => self.handle_unimplemented(req, "Issue certificate").await,
            "cert.import" => self.handle_unimplemented(req, "Import certificate").await,
            "cert.delete" => self.handle_unimplemented(req, "Delete certificate").await,
            "cert.renew" => self.handle_unimplemented(req, "Renew certificate").await,
            // Reverse Proxy
            "proxy.list" => self.handle_unimplemented(req, "List proxy rules").await,
            "proxy.create" => self.handle_unimplemented(req, "Create proxy rule").await,
            "proxy.update" => self.handle_unimplemented(req, "Update proxy rule").await,
            "proxy.delete" => self.handle_unimplemented(req, "Delete proxy rule").await,
            // VPN
            "vpn.profiles" => self.handle_unimplemented(req, "List VPN profiles").await,
            "vpn.profile.create" => self.handle_unimplemented(req, "Create VPN profile").await,
            "vpn.profile.update" => self.handle_unimplemented(req, "Update VPN profile").await,
            "vpn.profile.delete" => self.handle_unimplemented(req, "Delete VPN profile").await,
            "vpn.status" => self.handle_unimplemented(req, "VPN status").await,
            "vpn.connect" => self.handle_unimplemented(req, "Connect VPN").await,
            "vpn.disconnect" => self.handle_unimplemented(req, "Disconnect VPN").await,
            // Power
            "power.shutdown" => self.handle_unimplemented(req, "Shutdown").await,
            "power.reboot" => self.handle_unimplemented(req, "Reboot").await,
            "power.schedule.list" => self.handle_unimplemented(req, "List power schedules").await,
            "power.schedule.update" => self.handle_unimplemented(req, "Update power schedule").await,
            "power.wol.send" => self.handle_unimplemented(req, "Send Wake-on-LAN").await,
            // Time
            "time.get" => self.handle_unimplemented(req, "Get system time").await,
            "time.update" => self.handle_unimplemented(req, "Update system time").await,
            "time.ntp.get" => self.handle_unimplemented(req, "Get NTP settings").await,
            "time.ntp.update" => self.handle_unimplemented(req, "Update NTP settings").await,
            // Hardware
            "hardware.sensors" => self.handle_unimplemented(req, "Hardware sensors").await,
            "hardware.fans" => self.handle_unimplemented(req, "Fan status").await,
            "hardware.led.update" => self.handle_unimplemented(req, "Update LED").await,
            "hardware.ups.get" => self.handle_unimplemented(req, "Get UPS settings").await,
            "hardware.ups.update" => self.handle_unimplemented(req, "Update UPS settings").await,
            // Audit
            "audit.events" => self.handle_unimplemented(req, "Audit events").await,
            "audit.export" => self.handle_unimplemented(req, "Export audit").await,
            // Antivirus
            "antivirus.status" => self.handle_unimplemented(req, "Antivirus status").await,
            "antivirus.scan" => self.handle_unimplemented(req, "Run antivirus scan").await,
            "antivirus.signatures.update" => self
                .handle_unimplemented(req, "Update antivirus signatures")
                .await,
            "antivirus.quarantine.list" => self
                .handle_unimplemented(req, "List quarantined items")
                .await,
            "antivirus.quarantine.delete" => self
                .handle_unimplemented(req, "Delete quarantined item")
                .await,
            // System Config
            "sys_config.get" => self.handle_unimplemented(req, "Get config key").await,
            "sys_config.set" => self.handle_unimplemented(req, "Set config key").await,
            "sys_config.list" => self.handle_unimplemented(req, "List config keys").await,
            "sys_config.tree" => self.handle_unimplemented(req, "Config tree").await,
            "sys_config.history" => self.handle_unimplemented(req, "Config history").await,
            // Scheduler
            "scheduler.status" => self.handle_unimplemented(req, "Scheduler status").await,
            "scheduler.queue.list" => self.handle_unimplemented(req, "Scheduler queue").await,
            "scheduler.task.list" => self.handle_unimplemented(req, "Scheduler tasks").await,
            "scheduler.task.cancel" => self.handle_unimplemented(req, "Cancel scheduler task").await,
            // Node / Daemon
            "node.list" => self.handle_unimplemented(req, "List nodes").await,
            "node.get" => self.handle_unimplemented(req, "Node detail").await,
            "node.services.list" => self.handle_unimplemented(req, "Node services").await,
            "node.restart" => self.handle_unimplemented(req, "Restart node").await,
            "node.shutdown" => self.handle_unimplemented(req, "Shutdown node").await,
            // Activation
            "node.activate" => self.handle_unimplemented(req, "Activate node").await,
            "node.activation.status" => self.handle_unimplemented(req, "Activation status").await,
            // Task Manager
            "task.list" => self.handle_unimplemented(req, "List tasks").await,
            "task.get" => self.handle_unimplemented(req, "Task detail").await,
            "task.cancel" => self.handle_unimplemented(req, "Cancel task").await,
            "task.retry" => self.handle_unimplemented(req, "Retry task").await,
            "task.logs" => self.handle_unimplemented(req, "Task logs").await,
            // Verify Hub
            "verify.status" => self.handle_unimplemented(req, "Verify hub status").await,
            "verify.sessions" => self.handle_unimplemented(req, "List sessions").await,
            "verify.session.revoke" => self.handle_unimplemented(req, "Revoke session").await,
            // Repo Service
            "repo.sources" => self.handle_unimplemented(req, "List repo sources").await,
            "repo.pkgs" => self.handle_unimplemented(req, "List repo packages").await,
            "repo.install" => self.handle_unimplemented(req, "Install package").await,
            "repo.publish" => self.handle_unimplemented(req, "Publish package").await,
            "repo.sync" => self.handle_unimplemented(req, "Sync repo").await,
            "repo.tasks" => self.handle_unimplemented(req, "Repo tasks").await,
            // Message Bus
            "msgbus.status" => self.handle_unimplemented(req, "Message bus status").await,
            "msgbus.topics" => self.handle_unimplemented(req, "List topics").await,
            "msgbus.publish" => self.handle_unimplemented(req, "Publish message").await,
            // Nginx / Web Gateway
            "nginx.status" => self.handle_unimplemented(req, "Nginx status").await,
            "nginx.sites" => self.handle_unimplemented(req, "List sites").await,
            "nginx.site.update" => self.handle_unimplemented(req, "Update site").await,
            "nginx.reload" => self.handle_unimplemented(req, "Reload nginx").await,
            // K8s Service
            "k8s.status" => self.handle_unimplemented(req, "K8s status").await,
            "k8s.nodes" => self.handle_unimplemented(req, "List k8s nodes").await,
            "k8s.deployments" => self.handle_unimplemented(req, "List deployments").await,
            "k8s.deployment.scale" => self.handle_unimplemented(req, "Scale deployment").await,
            // Slog Server
            "slog.status" => self.handle_unimplemented(req, "Slog status").await,
            "slog.streams" => self.handle_unimplemented(req, "List log streams").await,
            "slog.query" => self.handle_unimplemented(req, "Query logs").await,
            // Gateway / Zone
            "gateway.status" => self.handle_unimplemented(req, "Gateway status").await,
            "gateway.routes.list" => self.handle_unimplemented(req, "List routes").await,
            "gateway.routes.update" => self.handle_unimplemented(req, "Update routes").await,
            "zone.info" => self.handle_unimplemented(req, "Zone info").await,
            "zone.config.get" => self.handle_unimplemented(req, "Get zone config").await,
            "zone.config.update" => self.handle_unimplemented(req, "Update zone config").await,
            "zone.devices.list" => self.handle_unimplemented(req, "List zone devices").await,
            // RBAC / Permission
            "rbac.model.get" => self.handle_unimplemented(req, "Get RBAC model").await,
            "rbac.model.update" => self.handle_unimplemented(req, "Update RBAC model").await,
            "rbac.policy.get" => self.handle_unimplemented(req, "Get RBAC policy").await,
            "rbac.policy.update" => self.handle_unimplemented(req, "Update RBAC policy").await,
            // Runtime
            "runtime.info" => self.handle_unimplemented(req, "Runtime info").await,
            "runtime.reload" => self.handle_unimplemented(req, "Reload runtime config").await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

#[async_trait]
impl HttpServer for ControlPanelServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "control-panel".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_control_panel_service() {
    let control_panel_server = ControlPanelServer::new();
    // Bind to the default control-panel service port.

    let runner = Runner::new(CONTROL_PANEL_SERVICE_PORT);
    // 添加 RPC 服务
    let _ = runner.add_http_server(
        "/kapi/control-panel".to_string(),
        Arc::new(control_panel_server),
    );

    //添加web
    //web_dir是当前可执行文件所在目录.join("web")
    let web_dir = std::env::current_exe().unwrap().parent().unwrap().join("web");
    let _ = runner.add_dir_handler(
        "/".to_string(),
        web_dir,
    ).await;

    let _ = runner.run().await;
}

async fn service_main() {
    init_logging("control-panel", true);
    let _ = start_control_panel_service().await;
    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
