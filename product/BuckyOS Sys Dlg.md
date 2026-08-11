# BuckyOS Sys Dlg

System Dialog 是系统提供的一组通用功能dlg,参数稳定，任何应用都可以通过触发调用来实现功能。 有2种使用方法
1）直接跳转（在新窗口或popup windows中打开)
    注意此时的完整URL看起来是整个Desktop的URL，但实际上只有背景 + SysDlg的内容，不会加载完整桌面
2）用iframe页面内拉起(适合应用内部集成)

使用上，分模态（期待返回值）和非模态（只拉起不关心最终结果）

简单列表如下

## sysdlg/app_installer

安装 App。固定以模态方式拉起，期待返回结果。

App Installer 是公开安装入口，负责校验和归一化调用方传入的安装来源、创建或恢复可持久化的安装任务，并完成信任校验、安装确认、进度展示和结果反馈。App Service 可以调用该入口，但不是安装来源解析或任务创建的必经层。

### 公开拉起参数

合法请求分为两种互斥形态，必须且只能提供 `task_id`、`identifier` 中的一个：

```ts
type AppInstallerLaunchParams =
  | {
      task_id: string
    }
  | {
      identifier: string
      ref?: string
      options?: AppInstallerLaunchOptions
    }

interface AppInstallerLaunchOptions {
  target?: {
    node_did?: string
    node_id?: string
  }
  install_params?: Record<string, unknown>
  offline?: boolean
}
```

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `task_id` | string | 条件必填 | 恢复已有安装任务。值为正 `i64` 的十进制字符串；对应任务的 `task_data_type` 必须为 `app.install`。不得与其它参数同时出现。 |
| `identifier` | string | 条件必填 | 创建标识符安装任务。可以是 App DID、名称、App Document Object ID、App Document URL、`.pikg` Object ID、`.pikg` URL 或协议支持的分享对象文本。 |
| `ref` | string | 否 | 推荐人或分享者标识，只能与 `identifier` 一起使用。创建任务时归一化为 `InstallSource::Identifier.referrer`。它只描述来源，不能提高候选内容的信任等级。 |
| `options` | object | 否 | 安装初始建议，只能与 `identifier` 一起使用。调用方提供的值仍须由 Installer 校验并向用户展示，不能视为用户批准。 |

`options.target` 只能用 `node_did` 或 `node_id` 选择目标 Node，两者同时提供时必须指向同一节点；目标的 `os`、`arch`、内核和 Runtime 信息由 Installer 从 Node 信息中取得，不能由调用方声明。`install_params` 必须是 JSON object。`offline = true` 表示禁止网络 Acquisition，不表示跳过 DID 信任或内容校验。

`policy`、`auto_confirm` 和 `user_id` 不属于公开拉起参数：安装策略由 Installer 根据入口、调用方身份和系统策略确定；普通调用方不能请求 `LOCAL_DEVELOPER` / `SYSTEM_INTERNAL` 或跳过用户确认；安装目标用户固定为当前已认证用户，代用户安装必须走独立的管理员授权接口。未知参数和未知 `options` 字段必须拒绝，不能静默透传到任务数据。

### URL 与 Dialog SDK 示例

新建标识符安装任务：

```text
/sysdlg/app_installer?identifier=$ENCODED_APP_IDENTIFIER&ref=$ENCODED_REFERRER
```

恢复已有任务：

```text
/sysdlg/app_installer?task_id=12345
```

iframe / Dialog SDK 直接传递等价的结构化参数，例如：

```json
{
  "identifier": "did:bns:filebrowser.buckyos",
  "ref": "did:bns:store.buckyos",
  "options": {
    "target": { "node_id": "ood-primary" },
    "offline": false
  }
}
```

URL 方式携带 `options` 时，值为一次 percent-encode 后的 JSON object 字符串；禁止 double decode。URL 不适合承载大型 `install_params`，复杂参数应使用 Dialog SDK，或在 Installer 的确认阶段由用户填写。

### 校验与任务构造

App Installer 收到参数后按以下顺序处理：

1. 校验参数结构、互斥关系、字段类型、大小限制和当前调用方权限；URL query 中同名参数重复出现必须拒绝。
2. `task_id` 入口加载已有 TaskManager 任务，校验任务存在、类型为 `app.install`，且当前用户有权读取和操作；该入口不得创建新任务或覆盖原始 request。
3. `identifier` 入口先识别输入类型并完成不产生安装副作用的最小归一化，再调用 `apps.install { identifier, referrer?, options? }` 创建任务。URL、Object ID 或分享对象暂时无法提取 App DID 时，后续 Resolve Stage 可以执行协议允许的最小 Acquisition，但在可信 Resolve 前不得进入 Inspect 或 Deploy。
4. 创建成功后取得 `task_id`，对话框切换为该任务的状态视图。直接跳转方式应使用 `history.replaceState` 将地址归一化为 `?task_id=...`，避免刷新页面重复创建任务。
5. 后续 Resolve、Inspect、Acquire、Verify、Prepare、Deploy、Activate 的全部状态以 TaskManager 中的 `AppInstallTaskData` 为真相源。

`staging_handle`、本地文件路径和文件内容都不属于公开拉起参数。本地 `.pikg` 由 BuckyOS Desktop、文件关联处理器或其它可信本地组件通过受控上传通道取得 `staging_handle`，在本地调用 `apps.install_package { staging_handle, options? }` 完成校验并创建任务，然后只使用返回的 `task_id` 拉起 App Installer。公开调用方不能读取、传递或复用 `staging_handle`。

`task_id` 标识从 Resolve 到 Activate 的同一个安装事务。关闭对话框只会让任务转到后台，不会取消任务；再次用同一个 `task_id` 拉起时恢复当前 Stage、确认状态、进度和结果。取消安装必须由用户在 Installer 中执行明确的取消操作。

安装协议中的 Web-to-Native jump URL 可以原样携带 `identifier` 和可选 `ref` 跳转到当前 Zone 的本入口；参数校验和任务创建由 App Installer 完成，不要求先经过 App Service。

## sysdlg/share

分享NamedObject （文件、对象...）

## sysdlg/select

选择一个系统中已经保存的 File / NamedObject

## sysdlg/request_do

请求执行一个action: 授权、支付、签名等
