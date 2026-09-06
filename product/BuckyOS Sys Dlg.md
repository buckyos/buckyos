# BuckyOS Sys Dlg

System Dialog 是系统提供的一组通用功能dlg,参数稳定，任何应用都可以通过触发调用来实现功能。 有2种使用方法
1）直接跳转（在新窗口或popup windows中打开)
    注意此时的完整URL看起来是整个Desktop的URL，但实际上只有背景 + SysDlg的内容，不会加载完整桌面
2）用iframe页面内拉起(适合应用内部集成)

使用上，分模态（期待返回值）和非模态（只拉起不关心最终结果）

简单列表如下

## sysdlg/app_installer

安装 App。以模态方式拉起；独立 URL 只加载背景和 Installer，不要求先打开 App Service。协议依据是 [App 安装协议 beta 2.2](../doc/App%20安装协议.md)，原型模型见 [UI DataModel](../src/frame/desktop/src/app/app-service/UI_DATAMODEL.md)。

### 公开拉起参数

```ts
type AppInstallerLaunchParams =
  | { task_id: string }
  | { identifier: string; ref?: string; options?: AppInstallerLaunchOptions }

interface AppInstallerLaunchOptions {
  target?: { node_did?: string; node_id?: string }
  install_params?: Record<string, unknown>
  offline?: boolean
}
```

必须且只能提供 `identifier` 或 `task_id`。所有重复 query key、未知 query key、未知 options 顶层字段、两种形态混用都被拒绝。URL 参数只 decode 一次。

| 参数 | 限制与语义 |
|---|---|
| task_id | 1–256 字符，不含空白或控制字符的不透明字符串；当前 TaskManager 生成 `t-<uuid>`。不转数字、不施加 i64 上限。只恢复已有 app.install/v1 或 app.update/v1 任务，不创建替代任务 |
| identifier | 1–32768 字符的应用标识；AppDID、名称、Object ID 或链接先进入来源检查/导入。链接不是 App Meta，当前未接入的导入能力必须明确报错。公开入口不接受文件内容、AppDoc JSON、staging handle 或主机路径 |
| ref | 1–2048 字符的来源说明，仅和 identifier 一起使用；不提高信任等级、不表示 owner 或管理员授权 |
| options | URL 中为一次 percent-encode 的 JSON 对象，序列化后最多 16384 字符；仅是初始建议，不是批准 |
| options.target | node_id/DID 来自真实节点清单，至少一个；同时提供必须匹配。OS/arch 来自节点信息，调用方不能自填 |
| options.install_params | JSON 对象；当前 Mock 仅投影受支持的 auto_start 建议，未支持建议不伪装成已生效，不直接透传至 Task。完整字段映射留待集成 |
| options.offline | 禁止网络获取，仍要求信任、内容、平台与配置通过 |

`policy`、`auto_confirm`、`user_id` 不属于公开参数。不能从链接请求 LOCAL_DEVELOPER / SYSTEM_INTERNAL 或绕过确认。安装归属来自已认证账号；原型使用明确的 Mock 身份上下文。敏感配置不能放入 URL，应在 Installer 内输入。

```text
/sysdlg/app_installer?identifier=did%3Abns%3Anextcloud.buckyos
/sysdlg/app_installer?task_id=t-0123456789abcdef0123456789abcdef
```

### 公开流程和恢复

1. 验证结构、长度、互斥关系与用户权限。
2. `task_id` 只加载授权范围内的已有任务。缺失、无权限、类型不支持必须显示错误，不创建新任务。
3. `identifier` 先完成来源导入与 `apps.inspect`。此时是检查草稿，没有持久执行任务；InstallPlan 中即便预留了 task_id，也不代表任务已经存在。
4. 展示校验、计划及当前 readiness。相同内容已满足时可直接查看应用，不新增任务；升级显示版本与影响；降级拒绝。
5. 配置修改后重新 Inspect；用户明确确认最新计划并完成 sudo 授权，才向 `apps.submit` 提交批准的 plan fingerprint 和幂等键。
6. submit 可返回 `satisfied` 与 `task_id:null`，也可返回新任务身份。只有实际创建任务后，独立入口才使用 history.replaceState 归一化为 `?task_id=...`。
7. 检查草稿刷新时重新 Inspect，敏感输入重填，不恢复授权。任务刷新时恢复执行、Plan/AppDoc 展示快照与安装结果；其中完整恢复快照目前是 Mock 定义，真实 status 接口需扩充。
8. 失败 retry 可能产生新 task_id 与 retry_of；暂停 resume 可沿用原编号。始终跟随返回值，重试后 URL 也跟随。

关闭检查页是退出准备并释放引用；关闭已提交任务是后台执行。取消必须显式执行，并受 `desired_state_committed` 边界限制；提交配置后不再承诺撤销。Completed 与节点部署、健康启动分开显示。

### 内部导入上下文

App Service、文件关联处理器、文件浏览器与 NDM 导入方负责受控字节导入。现有后端提供 `apps.staging.finalize/status/release`，之后以 `LocalPikg { staging_handle }` 参与 inspect/submit。不能在准备阶段调用旧安装入口来提前创建执行任务。

原型内部调用可以使用 `AppInstallerInternalParams = AppInstallerLaunchParams | { draft_id: string }`，它引用同一、按 Zone/用户隔离的 Store 中的来源草稿。此字段仅在可信进程内传递，**不是公开 URL/iframe 参数**；公开查询传 draft_id 会被拒绝。内部引用不包含路径或字节，不向外部调用方回传 staging handle。

本地来源不必先创建任务再拉起 Installer。检查、授权和任务提交都发生在公共 Installer 的统一流程中，来源页只交付来源上下文。

### Dialog 返回值

```ts
type AppInstallerDialogResult =
  | { action: 'background' }
  | { action: 'change-source' }
  | { action: 'close' }
  | { action: 'view-app'; serviceId: string }
```

`serviceId` 对普通应用是完整 AppInstanceId。返回值不包含密码、sudo token、敏感环境变量、文件内容或内部引用。跨来源通信的 origin/权限验证属于真实 Dialog SDK 集成，不把这个 React Mock 回调宣称为已完成的跨域协议。

### 已知集成缺口

截至 `4f22b93a`：Control Panel 不抓取客户端 URL；Object ID 需要本地内容；签名封装仍需 detached envelope 与 PIKG reader 对齐；安装入口尚需强制 sudo 授权闭环；status 尚缺完整 Plan/AppDoc/运行结果快照；Cancel capability 需按提交边界修正；运行就绪须聚合匹配 deployment 的节点证据。其余全量管理、Settings 保存和诊断缺口见 [App Service PRD](app_service/BuckyOS_App_Service_PRD.md)。本轮不实现这些后端功能。

## sysdlg/share

分享NamedObject （文件、对象...）

## sysdlg/select

选择一个系统中已经保存的 File / NamedObject

## sysdlg/request_do

请求执行一个action: 授权、支付、签名等
