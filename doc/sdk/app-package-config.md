# App 包的权限、端点与运行配置

> 当前权威：`dapp_meta/app.json` 中 `permissions` 和 `service_config_tips` 的声明形状及消费边界。本文替代旧 `notepads/service_config_tips + permissions 实用配置指南.md` 的使用说明；旧稿的目标设计见 [历史参考](../pikg/history/app-config-and-publication.md)。

开发态根 schema 只在 [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) 维护；AppDoc 和 InstallPlan 协议只在 [App 安装协议](../App%20安装协议.md) 维护。下面依据 TypeScript `validatePermissions/validateServiceConfigTips`、Rust `ServiceConfigTips` 及安装配置合成实现整理，不承诺 schema 能表达的字段都已具有完整运行时 enforcement。

## 1. 声明到运行的边界

```text
app.json 声明 → PIKG build 的 AppDoc → Inspect + InstallParams
→ 冻结的 InstallPlan / ServiceSpecConfig → Scheduler → node-daemon
```

`pikg init` 默认生成空 tips，不表示不支持配置。应在 build 前编辑 `app.json`；不要修改 `dapp_dist/APPDOC.json`。权限声明是申请上限，安装时的选择和目标授权另行决定；元数据解析成功不授予额外资源访问权。

## 2. `permissions`

```json
{
  "permissions": [
    {"scope_path": "user/home", "required": true, "actions": ["read"], "exp": null}
  ]
}
```

每项仅含 `scope_path/required/actions/exp`：scope 必须为非空字符串，required 必须为 boolean，actions 可省略并使用字符串数组，exp 可省略或为 null/uint32。有效 scope 与授权行为由 [permission.rs](../../src/kernel/buckyos-api/src/permission.rs) 和 Installer 决定。

不要把端口或存储路径当作 permission，也不要因声明了 permission 就假定 mount 已创建。具体安装选择必须是声明范围内的有效子集。

## 3. HTTP 服务端点

例如服务实际监听 `0.0.0.0:3000` 时，在 `service_config_tips` 中声明：

```json
{
  "service_endpoints": {
    "www": {
      "protocol": "http",
      "inner_port": 3000,
      "required": true,
      "expose": {"route": {"type": "web"}, "scope": "", "allow_guest": false}
    }
  }
}
```

`protocol` 为 http/https/tcp/udp；`inner_port` 与服务监听端口保持一致。description 是语言到字符串的 map。expose route 可为 web，或 `{"type":"port","preferred_port":3000}`；scope 和 allow_guest 可省略。普通用户的最终 hostname/port 来自安装和 Registry 分配，不能由展示名自行拼接认定。

## 4. 其他 tips 的形状

| 字段 | 当前解析形状 |
| --- | --- |
| `data_mount_points` / `local_cache_mount_points` / `external_mount_points` | map；每项为 null 或 `{mount_point_name, access, reason}`，reason 为语言字符串 map |
| `rdb_instances` | map；每项为 `{backend, version?, schema?, connection?}`，backend 为 sqlite/postgres，schema 为对应 backend 的字符串 map |
| `instance_volume` | `{mode?, quota_mib?, ephemeral_contents?}`；mode 为 required/optional/disabled，contents 为相对内容说明数组 |
| `bash_envs` | map；每项 `{required, description?}`，description 为语言字符串 map |
| `runtime_caps` | 字符串 map |
| `container_param` / `start_param` | 可选字符串 |
| 其他 tips 键 | 自定义配置，可参与内容 identity；不会因此自动得到新的 runtime 能力 |

嵌套的 endpoint/mount/database/env/volume 对象拒绝各自 schema 之外的字段。Mount access 的常用值为 `read_only/read_write/read_write_append`；它是声明字符串，append-only enforcement 不能仅凭 schema 通过推断。`instance_volume.quota_mib` 在当前 Rust 类型中仍为未强制执行的软配额。

## 5. 挂载与数据的实际使用

`data_mount_points` 和 `external_mount_points` 的声明需要与 `InstallParams` 中的明确选择一起由 deployer 合成；不得把旧稿中“null 自动生成标准 AppData”“任意环境变量会自动展开”的目标语义当作现有合同。部署只消费最终计划配置，不在安装后随意修改生成的 ServiceSpecConfig。

应用自身持久数据应使用运行环境给出的 `BUCKYOS_DATA_DIR`，其宿主机位置按 owner/AppId 隔离，跨应用版本保持稳定；完整目录与容器映射见 [path_usage](../path_usage.md)。不要把缓存、instance volume 和用户持久数据混为一个生命周期。

`rdb_instances` 可被 schema 接受不表示任意 Node 服务或第三方镜像都能调用 Rust RDB API、取得连接信息或访问数据库落点。`bash_envs/runtime_caps/start_param/container_param` 同样需要核对目标 AppLoader 的实际消费和权限；旧配置稿中的未落地能力已移出当前指南。

## 6. 参考实现与验证

- [pikg_protocol.ts](https://github.com/buckyos/buckyos-websdk/blob/main/cli/modules/pikg_protocol.ts)：打包侧字段校验。
- [app_doc.rs](../../src/kernel/buckyos-api/src/app_doc.rs)：Rust AppDoc/tips 类型。
- [app_install_planner.rs](../../src/frame/control_panel/src/app_install_planner.rs)、[app_install_deployer.rs](../../src/frame/control_panel/src/app_install_deployer.rs)：权限选择和最终配置合成。
- [app_loader.rs](../../src/kernel/node_daemon/src/app_loader.rs)：运行时数据路径、mount/env/container 消费。

最小场景是：编辑开发态配置、build/pack/info、用计划首装、核对实际端点和数据目录，再升版本检查数据保留。命令和已配置 DV 的准备见 [应用开发快速入门](app-dev-quickstart.md)。本页只经源码核对，不代表对所有字段和平台完成了运行验收。
