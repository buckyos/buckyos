# 应用 Upload File / Folder 统一改进 TODO

> Review 日期：2026-10-22。范围：当前工作区源码，主仓库 HEAD `c6a021f42`；同时核对同级 `buckyos-websdk`（`4786dc8`）、`filebrowser`（`99e9a5bb`）及 BuckyOSApp。
> 实施更新：2026-10-23。本轮已跨 `buckyos`、`buckyos-websdk`、`cyfs-ndn`、`filebrowser` 修改源码并执行本地服务/浏览器验证。第 2、3 节保留实施前 review 快照，不能据此判断当前实现；第 4、5 节记录实现状态，第 7 节记录实际验收与未验证边界。

## 1. 目标与结论

系统 SDK 的目标是 **named-object first、尽早去重**：先识别命名对象并尽可能复用已有内容，只传输缺失的数据，最终交付可使用的 ObjId。`sys_test` 的职责是演示这套 SDK 标准用法；通用能力归 SDK/NamedStore，不应靠复制 demo 的应用私有逻辑实现。

通用内容入口应默认同时提供 **Upload files / Upload folder**。各应用统一接入上述对象导入流程；传统的「把浏览器字节流拷到应用上传接口/目标文件，再计算对象」路径应随迁移收敛。TUS、分块传输只是补齐缺失内容的底层机制。

### 1.1 标准流程与职责边界

```text
选择 file / folder
  → SDK 尽早查询 QCID / 可复用内容，命中则避免全量读取、重复哈希和上传
  → 未命中时计算必要的内容标识，构造 FileObject / ChunkList / DirObject
  → 按对象及 chunk 去重，仅补齐缺失内容与元数据
  → NamedStore 中对象可用，SDK 交付 ObjId
      ├─ 应用只需要对象：直接保存、传递或引用 ObjId
      └─ 应用需要实体文件：在目标端以 NamedStore + ObjId 本地提取文件/目录
```

- **去重放在昂贵操作之前。** 支持 QCID 的文件先做轻量查询；命中时复用内容标识，按需生成当前文件名/目录结构的对象。未命中再做必要的完整哈希，并复用哈希结果；目录中已命中的叶文件和已存在的 chunk 不重复传输。小文件等不适用 QCID 的情况直接走对象/chunk 标识查重。
- **对象标识与就绪状态分开。** 稳定 ObjId 计算出来后可尽早给 UI 或业务准备阶段使用；ObjId 已知不等于内容已经可用。SDK 明确报告是否还需补齐内容、对象是否已就绪；应用不必等本地实体文件生成才能得到或使用对象引用。
- **应用消费对象结果。** 只需要 ObjId 的应用到 SDK 对象就绪即可完成导入，无需创建实体文件，也无需自建一套登记/上传后端。消息、发布等原有业务提交按自身需要建立引用和 retention。
- **实体文件是对象的后续产物。** 目标端通过 NamedStore 按 ObjId 本地提取；目标路径、ACL、覆盖、冲突和原子替换仍属于业务。该步骤不要求浏览器再次上传相同字节，也不另建网络 stream-copy 上传协议。文件/目录读取与提取优先复用 NamedStore 能力。
- **demo 跟随 SDK。** 缺失的去重、对象登记和生命周期能力先补到 SDK/NamedStore，再由 `sys_test` 以最少业务代码展示 file/folder、秒传、缺失内容上传，以及「只取 ObjId / 本地提取」两种消费方式。

实施前发现的问题不只是缺少文件夹按钮：

- `sys_test` 已演示单文件的分块对象登记、内容完整性检查及 QCID 注册顺序，但还没有完整的文件夹示例；当前 demo 中手写的通用步骤应回归 SDK。
- MessageHub 已走 NDM，但文件夹被展开为多个文件；Desktop Files 和独立 FileBrowser 已有 file/folder 双入口，却分别走 NFSP、自有资源上传协议。
- HomeStation、AIWorkspace 各有独立上传接口；部分入口实际只是本地读取或小图配置，不能一律改造成通用目录上传。
- **字节上传完成、对象元数据完整、业务接收成功、接收者可读是四件事。** SDK 负责对象就绪，应用按消费方式决定后续动作。当前读侧仍有单 chunk 限制，单改 picker 或放宽大小限制会产生不可用的附件。

## 2. 实施前快照：`sys_test` 演示与 SDK 能力缺口

源码入口：[前端上传](../src/apps/sys_test/web/index.ts)、[AppService](../src/apps/sys_test/main.ts)、[WebSDK NDM](../../buckyos-websdk/src/ndm_client.ts)。以下行号用于定位本次 review 快照。

| 阶段 | 当前实现和正确约束 |
| --- | --- |
| 身份与 endpoint | `web/index.ts:744` 校验 runtime 的 AppId / AppInstanceId / owner / Zone 后初始化；`main.ts:211` 起取得 AppService session。实际授权以验证后的用户及 App/System target 为准，不能靠 URL、客户端 metadata 或硬编码 app 名判断。 |
| 选择与导入 | `web/index.ts:551` 调 `pickupAndImport({mode:'single_file', autoStartUpload:false})`，提供哈希、QCID、缩略图进度，区别用户取消与失败。 |
| 查重与上传 | `web/index.ts:509、644` 区分 `on_cache`、`all_in_store`、`not_required`；未命中时 `startUpload` 后轮询到终态。`startUpload` 返回不代表上传已完成。 |
| 对象登记 | `main.ts:1150` 起先登记 ChunkList，再登记 FileObject；查对象、内容和各 chunk 状态。WebSDK `startUpload` 只上传文件 chunks，不自动登记 FileObject / ChunkList / DirObject。 |
| 完整性与 QCID | `main.ts:1247、1308` 以 `isObjectStored(contentId)` 的 fully stored 结果控制 `addChunkBySameAs`。QCID lookup 命中、对象可查、内容递归完整必须分开表示。 |
| 对象消费 | 示例止于诊断查询。SDK 对象就绪后，应用可以直接使用 ObjId；需要业务引用或实体路径时，再分别提交引用或从 NamedStore 本地提取。不能把字节上传终态当对象就绪，也不要求每个应用都落地实体文件。 |

需要补强的参考实现/SDK 缺口：

- `sys_test/web/index.ts:552` 仅有 `single_file`。SDK 虽已有 `single_dir` 和目录 ID 计算，但 `ImportedDirObject` 只暴露 children，计算后没有像 `_ndnFileObject` 一样保留 canonical DirObject；见 `ndm_client.ts:122、1217`。应用目前不能直接取一份完整待登记对象清单。
- SDK 目录依赖 `FileList/webkitRelativePath` 重建；空子目录不可见，纯空目录被当成 `USER_CANCELLED`（`ndm_client.ts:1158、1365`）。所有叶文件命中 `all_in_store` 也不代表新 DirObject 已登记。
- 示例自行按 32 MiB 再次读取文件计算 ChunkList（`web/index.ts:23、51、613`），SDK 实际默认分块也是 32 MiB（`ndm_client.ts:438`）。这会使已命中内容或已完成哈希的文件再次被完整读取；应由 SDK 复用对象/分块结果，demo 和应用不复制此逻辑。
- `/ndm_query` 将登记/查询错误放入子结果，最终仍返回 `ok:true`（`main.ts:1377`）；前端按顶层 `ok` 显示成功（`web/index.ts:714`）。诊断请求成功不能作为上传成功判据，应增加明确的对象完成结果，并展示子步骤失败。
- 示例轮询没有超时/取消；SDK session 的 AbortController 存在于内部，但没有公开的 import-session 取消、释放、刷新恢复接口。服务端支持续传，不等于浏览器刷新后已经能恢复。
- Browser runtime 明确禁止直接调用 SDK structured Store API（`ndm_client.ts:587`）。补齐标准 SDK 导入流程所需的受限服务调用及授权交接，使应用直接得到对象结果；不要求每个应用自建 finalize 后端，也不让页面手写 `/store/put_object`。

本地提取的复用基础已存在：[NamedDataMgr](../../cyfs-ndn/src/named_store/src/ndm.rs):1059 的 `open_reader(ObjId, inner_path)` 支持 FileObject 的单 chunk / ChunkList 内容及目录内路径；结合 `get_object`（:456）、`get_dir_child`（:564）可遍历目录并提取。统一补齐需要的高层提取封装即可。注意 [ndn-toolkit/tools.rs](../../cyfs-ndn/src/ndn-toolkit/src/tools.rs):513、523 的 `restore_file_object / restore_dir_object` 当前仍返回 `Unsupported`，不能仅凭函数名当成可用实现。

## 3. 实施前快照：入口盘点

### 3.1 通用内容与附件

| 入口 | 当前 file / folder 能力及实现证据 | 改进落点 |
| --- | --- | --- |
| MessageHub | 有多文件、目录 picker 和递归拖拽提取。`ConversationComposer.tsx:190、245`、`attachmentDraft.ts:63` 保留 relativePath；[upload.ts](../src/frame/desktop/src/app/messagehub/api/upload.ts):55 使用全局 `ndm.setImportProvider`，以 `multi_file` 导入，:85 仅手工登记 FileObject，:95 将路径放到附件 name。目录未作为 DirObject 交付。 | 复用完整 NDM finalize；支持真正目录附件及目录浏览/下载；避免覆盖全局 picker provider；补取消、重试与目录批次进度。 |
| Desktop Files / File Browser | 已有 Upload files / Upload folder；[FileBrowserView.tsx](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx):518 按 `webkitRelativePath` 建目录并逐文件入队，:559 创建 picker。数据面走 [NFSP transfers](../src/frame/desktop/src/app/filebrowser/data/nfsp/transfers.ts) 和 `NfspClient` 的 probe / open_write / TUS / commit_file。 | 改为 SDK 对象导入/去重 → ObjId → 目标端 NamedStore 本地提取。NFSP 保留目标路径、ACL、revision/lease 和冲突提交职责，原上传流不作为另一条标准路径保留。 |
| HomeStation | [PublishComposer.tsx](../src/frame/desktop/src/app/homestation/publish/PublishComposer.tsx):24 只分 image/video/audio，未知类型回落 image；:70 上传，:76 丢失错误原因。经 [transport.ts](../src/frame/desktop/src/app/homestation/api/transport.ts):70、122 直接 PUT 整个 Blob；[http.rs](../src/frame/homestation/src/http.rs):24、133 限 1 byte～32 MiB；[publish.rs](../src/frame/homestation/src/publish.rs):1011 起写单 chunk、FileObject 及本用户业务记录。无 folder 入口。 | 图片/视频/音频仍按媒体语义处理；通用文件/目录分享补显式类型及入口，复用 NDM，并保留 `publish.create`、受众授权、对象登记和 retention。 |
| AIWorkspace | [session.ts](../src/frame/desktop/src/app/aiworkspace/api/session.ts):585 的 `asset.begin_upload → PUT → asset.finish_upload`，服务端 [lib.rs](../src/frame/aiworkspace/server/src/lib.rs):447、642 自管上传 session / staging；资源写入 workspace 私有 ObjectStore，当前资产默认上限 32 MiB。主要入口是图片资源、draw.io、workspace ZIP 导入，不是完整的通用文件夹导入。 | 通用资源导入补 file/folder，并将目录映射到 workspace 资源层级；媒体替换和专用格式导入保留类型约束。接 NDM 时仍需 workspace 授权、stage/commit 和离线引用映射。 |
| 独立 FileBrowser 应用（同级仓库） | 与 Desktop Files 是两个实现；[app.json](../../filebrowser/dapp_meta/app.json) 和构建脚本表明它是可打包的 BuckyOS App，未核实当前运行环境是否部署。[Upload.vue](../../filebrowser/frontend/src/components/prompts/Upload.vue):93 已有 file/folder，走自有 resources/TUS 与 Go 文件服务，不经过 NDM。 | 纳入跨仓库迁移，统一为 SDK 对象导入后按 ObjId 本地提取；保留目标目录、覆盖/重命名、权限与浏览器 fallback 语义。不要只改 Desktop 后宣称所有文件应用已统一。 |

**已确认的联动阻塞：**

1. MessageHub 当前主动限制附件 ≤32 MiB（[attachmentDraft.ts](../src/frame/desktop/src/app/messagehub/conversation/input/attachmentDraft.ts):8），上传代码没有 ChunkList 登记；[msg-center 读取](../src/frame/msg_center/src/object_access.rs):299–320 只接受 FileObject 且 `content` 必须是单 chunk。这是解除限制/新增目录的阻塞，不是已经允许大文件后才失败。
2. HomeStation [object_content](../src/frame/homestation/src/http.rs):320 也是 FileObject → 单 chunk；只换上传器会打断媒体读取。现有媒体 schema、发布附件限制、受众和对象读取必须一起改。
3. NFSP [op_commit_file](../src/frame/nfs_server/src/mutate.rs):584 只消费自己的 staging handle 或本地 `content_index` 哈希；`obj_id` 别名也会被当作普通 hash 解析，不是 NDM FileObject/DirObject 提取入口。这里需要衔接 NamedStore 按 ObjId 本地提取与目标路径提交，而非延续另一套上传协议；不能把 NDM 返回的 ID 直接塞进现有 `commit_file`。
4. AIWorkspace `take_upload` 只检查 kind/principal/received，未核对 begin 时的 workspace_id；finish 先删除 session/临时文件，再 stage（`server/src/lib.rs:463、650`）。同一用户可能误提交到另一有权 workspace，失败或成功响应丢失后无法重试同一个 upload_id。目标 workspace 仍有 ACL 校验，不能据此断言越权。
5. MessageHub [attachmentDraft.ts](../src/frame/desktop/src/app/messagehub/conversation/input/attachmentDraft.ts):192、203 将目录遍历错误转为 null / 空数组，可能漏文件而不报告；独立 FileBrowser [stores/upload.ts](../../filebrowser/frontend/src/stores/upload.ts):113 捕获错误后仍执行 `finishUpload`，整队列结束仍发成功反馈。两处都需明确区分全成功、部分失败、取消。
6. Desktop Files 的 [NFSP transfers](../src/frame/desktop/src/app/filebrowser/data/nfsp/transfers.ts):74 先将整文件读入内存并 SHA-256，再按 4 MiB 传输；当前三并发限制不能消除大文件内存峰值。目录拖拽在 `FileBrowserView.tsx:1106` 被明确拒绝，提示改用 folder picker，不能把 picker 支持等同于拖拽也支持。

### 3.2 需要保留业务边界的入口

| 入口 | 现状与处理原则 |
| --- | --- |
| AI Center Playground | [RequestFields.tsx](../src/frame/desktop/src/app/ai-center/components/playground/RequestFields.tsx):8、54 选单文件，≤20 MiB，FileReader 转 base64 送 AICC；已有手填 `named_object`。上传数据面应接 NDM 并生成资源引用；单图/单音频字段不加 folder，只有批量资源字段按 schema 接受目录。 |
| 用户/Agent 头像、Zone 图标、OpenDAN 头像 | Desktop [imageFile.ts](../src/frame/desktop/src/presentation/imageFile.ts) 使用有大小限制的 canvas/data URL；OpenDAN 是 profile 图片更新。保留单图选择、裁剪、预览、格式和尺寸校验，不强行目录化。若以后改用对象引用，再统一传输并验证头像读者可访问。 |
| Preview | [PreviewLanding.tsx](../src/frame/desktop/src/app/preview/PreviewLanding.tsx):25 将 File 映射为本地 Blob 预览；不是服务端上传，不为预览隐式写入 NDM。 |
| App 安装器 | [InstallWizard.tsx](../src/frame/desktop/src/app/app-service/pages/InstallWizard.tsx):75 是本地 `.pikg` 入口，但 [mock/store.ts](../src/frame/desktop/src/app/app-service/mock/store.ts):311 只处理小型 mock envelope，不能统计为真实 PIKG 上传。接真实安装时复用 NDM；文件夹需先有明确的包导入语义。 |
| Workflow / draw.io / workspace ZIP | Workflow [ImportDialog.tsx](../src/frame/desktop/src/app/workflow/components/ImportDialog.tsx):101 读本地 JSON，后续仍是 mock dry run；draw.io、ZIP 为格式导入。可以复用选取和错误处理，不把任意目录当作这些格式。 |
| 文本编辑器保存 | [documentStore.ts](../src/apps/text_editor/src/fs/documentStore.ts):88 通过 NFSP 写现有文件，属于编辑保存。统一数据面时回归，界面不加目录上传。 |

BuckyOSApp 本轮源码扫描未找到独立通用上传链路；内核 slog、Node Executor、Telegram 等后台协议传输不属于本次用户文件/目录 picker 改造。

## 4. P0：补齐 SDK 对象导入流程，由 `sys_test` 演示

- [x] **U-01：把 `sys_test` 收敛为 SDK 用法 demo。** 随 SDK 能力补齐多文件/文件夹演示，展示尽早查重、对象标识生成、缺失内容上传和对象就绪；演示只取 ObjId 及从 NamedStore 本地提取两个消费分支。移除 demo 自行二次哈希、拼装登记请求的通用逻辑；状态、取消和错误直接消费 SDK 结果。关键登记失败或内容未完整时不得标对象就绪，QCID 优化失败单独报告。
- [x] **U-02：完善现有 WebSDK 的对象导入和生命周期。** 统一执行轻量 QCID 查询、必要的哈希、对象/chunk 查重、缺失内容传输及元数据登记。FileObject / ChunkList / DirObject 的 canonical 序列化和待登记清单由 SDK 维护并复用，应用不读取私有字段再重算、重传。输出稳定 ObjId 和独立的就绪状态；已选 File/目录树可直接导入，避免覆盖全局 `setImportProvider`。按需要暴露取消、释放和失败重试，不引入另一套传输引擎。
- [x] **U-03：明确文件与目录交付模型。** 普通多文件是一批独立文件；Upload folder 保留选中根目录、嵌套路径、同名不同路径和目录身份。内容型业务提交 DirObject；文件系统目的地物化成真实目录。目录展开为媒体集合必须是明确的业务操作，不能静默丢失目录结构或自动压成 ZIP。
- [x] **U-04：定义空目录和能力降级。** 能枚举目录节点时保留空目录；只能拿 FileList 时明确限制，纯空目录给出准确提示。根据运行时能力展示 file/folder 双入口；移动端/不支持目录选择的浏览器保留文件上传并说明能力限制。拖拽、粘贴和 picker 最终进入同一导入路径，校验相对路径及文件/目录名称冲突。
- [x] **U-05：区分 SDK 对象就绪与应用消费。** SDK 对缺失对象按依赖顺序补齐、登记并验证强依赖完整；已存在的内容直接复用，跳过字节上传时仍处理缺失元数据，最终返回可用 ObjId。应用只需对象时直接消费此结果；需要业务引用时接管 retention，需要实体文件时按 ObjId 本地提取后完成路径提交。各阶段结果可重试，响应丢失不导致重复消息、重复发布或重复落盘；不把每种应用的业务 commit 塞进 SDK。
- [x] **U-06：打通身份和查重授权。** NDM holdings 按「用户 + canonical App/System target」隔离（[ndm/mod.rs](../src/frame/nfs_server/src/ndm/mod.rs):99），`put_object` 检查子对象归属（[forward.rs](../src/frame/nfs_server/src/ndm/forward.rs):94）。明确浏览器上传与后端 finalize 的授权交接，验证普通用户、非 owner AppInstance、跨入口秒传；不得用管理员 session 绕过，也不得信任 SDK TUS metadata 中固定的 `app_id:'default'`。查重命中不自动赋予对象读取/登记权限。
- [x] **U-07：统一按 ObjId 读取/本地提取与持久性。** 复用 NamedStore 的文件/目录读取及提取能力，支持 ChunkList 和目录树；需要实体文件的服务只衔接本地提取和自身路径提交，不从浏览器再次 stream copy。遵守业务读权限，保留消息和发布已有 retention。NDM 上传 holdings 的临时 lease（当前 24 小时，`ndm/mod.rs:45`）不能代替业务持久保留。取消/失败只清理该任务拥有的临时资源，不能删除其他引用共享的内容。
- [x] **U-07a：先修复会误报成功的入口。** MessageHub 目录枚举返回失败项，不能把读取失败当目录结束；独立 FileBrowser 的失败任务不补成成功进度，批次完成按成功/失败/取消计数。此项可独立修复，不必等待数据面迁移。

实施约束：标准对象导入归现有 WebSDK/NDM/NamedStore；NFSP 和其他业务服务只衔接对象引用或本地提取后的业务操作。本 TODO 不要求新增依赖或独立上传服务。涉及协议、公共类型和状态时，同步修改前后端、SDK 和文档，不新增旧协议兼容层。

## 5. P1：逐入口迁移

- [x] **U-08：MessageHub。** 消费 SDK 返回的就绪 ObjId，移除页面手写对象登记；补目录附件 schema、消息构造、接收端展示/遍历/下载，按目录整体跟踪进度和失败项。复用现有图片规范化、语音附件和发送失败处理；通用目录内的原始文件不应被媒体压缩流程无提示改写。完成 ChunkList 读写联调后才放宽 32 MiB 限制，检查重复发送与 `msg-center` retention。
- [x] **U-09：Desktop Files。** 浏览器统一调用 SDK 对象导入，得到 ObjId 后由目标端从 NamedStore 本地提取文件/目录；NFSP 衔接目标路径提交，保留 ACL、lease/revision、命名冲突、覆盖决策和列表失效通知。删除应用自己的整文件 SHA-256 / probe / 字节上传编排，复用 SDK 的早期去重和分块处理。补目录拖拽、批次取消及失败项重试，明确部分落盘时的恢复策略，不宣称目录原子提交。
- [x] **U-10：HomeStation。** SDK 完成对象导入后，应用接口接收 ObjId 并登记本用户记录/publish 引用；删除全量 Blob PUT 和应用自行构造上传对象的链路，无实体文件需求时直接引用对象。新增通用 file/dir 附件或明确的目录分享引用，联动 schema、发布、读取、受众和 retention；媒体入口继续按 image/video/audio 校验。错误保留原因，并支持取消、逐项重试和真实进度。
- [x] **U-11：AIWorkspace。** 对数据树和通用画布插入增加 file/folder 导入，目录映射为 `buckyos.container`、文件为 asset-ref，定义画布布局和业务批次；用统一上传结果接 workspace stage/commit。绑定 upload 与目标 workspace；提交幂等且结果可恢复；补临时资源过期清理。同步检查 AssetRef 读取/导出、离线 ObjectStore、资源 ID 映射和同步流程，不能仅将本地 asset ID 换成 NDM ID；图片替换、draw.io、workspace ZIP 以及扩展/快照派生上传按各自语义验收。当前离线 session 不允许上传，不把离线队列描述成已有上传恢复能力。
- [x] **U-12：独立 FileBrowser。** 在同级仓库把 resources/TUS 上传替换为 SDK 对象导入，服务端接收 ObjId、从 NamedStore 本地提取并提交目标路径。与 Desktop Files 分别验收 file/folder、覆盖、重命名、断线恢复和真实目标目录回读；业务路径权限仍由各自服务校验。
- [x] **U-13：AI Center 与未来真实 PIKG 安装。** 资源文件优先转为上传后可读的 `named_object`；AICC/provider 能正确读取并维持请求期引用后再替换 base64 默认路径。PIKG 先接真实包校验/导入，再接统一上传；不把现有 mock 演示当作完成。

依赖顺序：以 U-02、U-05、U-07 的 SDK/NamedStore 能力建立共同基线，U-01 跟随实现演示；U-08～U-13 可按应用并行，分别完成对象引用或本地提取的消费闭环。文件夹入口与大文件上限只在对应消费链路可用后开放。

## 6. P2：回归矩阵与验收

下表是完整验收矩阵；本地已运行的项目与结果见第 7 节。代码完成不等于真实 Zone、移动浏览器或外部 provider 已验收。

| 维度 | 必须覆盖的场景及成功判据 |
| --- | --- |
| 文件 | 单文件、多文件、0 字节、32 MiB 边界及跨 chunk 大文件；对不接受空文件的媒体入口明确拒绝。验证 ObjectId、大小、字节内容，不能只断言 HTTP 200。 |
| 目录 | 多层目录、同名不同路径、同名同内容的去重文件、中文/空格/特殊字符、空目录策略；根目录和全部叶文件可从业务入口回读，取消或部分失败有明确结果。 |
| 查重 | 首传、同入口重传、跨入口/不同 AppInstance 重传、部分文件命中、文件改名、内容相同但目录结构变化；命中后仍正确登记新元数据，读取/写入授权不扩大。记录 lookup 时机、全量读取/哈希次数和实际发送字节：查重发生在首次内容传输前，有效 QCID 命中不再完整读文件；完全命中时内容上传字节为 0，部分命中仅发送缺失数据，不能只凭 UI 显示“秒传”验收。 |
| 失败恢复 | 断网、TUS offset 冲突、token 过期、服务重启、页面刷新、用户取消、上传成功后登记失败、业务提交失败及响应丢失。可重试项能继续；不能恢复时准确提示重新选择，不能假称自动恢复。 |
| 权限 | 未登录、普通用户、目标只读、AppInstance/owner 不匹配、上传会话被其他用户访问、上传后目标权限撤销；最终提交重新验权。 |
| 生命周期 | 并行跨应用上传不串文件；目录进度在内容去重时仍能完成；成功后刷新/新会话可读；临时 lease 到期/GC 后正式引用仍可读；失败、取消、业务删除按引用释放资源。 |
| 消费端 | MessageHub 收件人、HomeStation 指定受众/访客、Files 目标目录、AIWorkspace 刷新及离线同步；每条都验证最终目录结构或附件下载内容。 |
| 两种结果 | 同一 ObjId 可直接作为应用引用，也可从 NamedStore 本地提取出一致的实体文件/目录。只取 ObjId 不触发实体落盘；提取不产生浏览器二次上传；更换目标路径仅重新执行本地提取/提交，不重传已有内容。 |
| 浏览器 | 桌面 picker、目录拖拽、粘贴、移动端及不支持目录选择的环境；能力降级和类型限制文案准确。 |

- [x] 扩展现有测试而非重新建测试框架。WebSDK 已有目录树/hash、大文件 materialize、mock TUS 鉴权用例，但 [真实浏览器上传用例](../../buckyos-websdk/tests/browser/real-browser/ndm_client_upload.spec.js) 仅覆盖小文件/多文件 completed/progress；补 folder、大文件、后端 finalize 和实际内容回读。
- [x] 复用已经存在的真实链路用例：[MessageHub real](../src/frame/desktop/tests/e2e/real/messagehub.real.spec.ts):195 已有单小文件上传、鉴权回读及慢网进度；[Files NFSP](../src/frame/desktop/tests/e2e/pages/filebrowser.nfsp.spec.ts):61 已有 opt-in 真实 NFSP 单文件上传。保留这些测试，再扩展目录、大文件、失败恢复；不能误记为「系统没有真实上传测试」。
- [x] 在 `sys_test` 增加可重复的 file/folder SDK demo，展示首次导入、有效 QCID 命中、部分内容命中、只取 ObjId 和按 ObjId 本地提取；失败断言验证 SDK 的对象就绪语义。
- [ ] 在更新后的真实 Zone 验收，记录账号角色、浏览器、SDK/服务版本、文件规模及实际上传字节。包括普通用户/非 owner AppInstance、跨入口与跨应用、目标权限撤销、真实 GC 后业务回读、移动端能力降级。
- [x] 扩展现有 [NDM 服务测试](../src/frame/nfs_server/tests/ndm.rs)、[NDM DV](../test/nfs_server_test/ndm_dv.ts)、[MessageHub 上传测试](../src/frame/desktop/tests/e2e/pages/messagehub-upload.spec.ts)、Files NFSP、HomeStation、AIWorkspace 的对应测试。mock 通过不能代替普通用户在真实 Zone 的上传与回读。
- [x] 实施后按影响范围执行 WebSDK 定向测试、Desktop `pnpm check` / `pnpm build` 和相关 Playwright；在 `src/` 下对改到的 `nfs_server`、`msg_center`、`homestation`、`aiworkspace` 运行定向 Rust 测试。跨仓库 FileBrowser 独立验证，不把主仓库通过当作应用已通过。
- [x] 同步维护 `sys_test/README.md`、WebSDK NDM 接口说明、各应用 UI_DATAMODEL 与协议文档；若影响 Agent/App 开发使用方式，检查 `buckyos_app_dev`、`buckyos_dev_release`、`opendan_agent_dev` 等实际引用手册。已更新 `buckyos_app_dev` 的 SDK 对象导入指引；已核对 `buckyos_dev_release`、`opendan_agent_dev`，其打包/发布与 Agent 命令接口未改变，不改写既有 CLI 流程。

完成标准：主要通用入口都有 file/folder 两种能力；SDK 提供 named-object-first、尽早去重的标准流程，`sys_test` 以最少业务代码演示；应用直接消费 ObjId，按需从 NamedStore 本地提取；重复内容不重传，成功对象可读；类型受限和本地预览入口的边界明确；旧的重复上传实现随迁移删除。

## 7. 实施结果与验收边界

### 7.1 当前实现

- WebSDK 负责 canonical manifest、QCID/内容/chunk 查重、TUS、依赖顺序 finalize、独立 ready 状态、取消/重试/释放与传输字节统计；调用方可直接传入 File 或目录树。纯空 FileList 的限制不隐藏，显式树保留空目录。
- NDM `/ndm/v1/import/finalize` 校验内容及授权，提供可重试登记；QCID 采样验证失败只作为优化 warning。lookup 只有 `reusable: true` 才能复用，跨用户/不同 canonical AppInstance 不因已知 ID 得到读取权。零字节创建即存入真实空 chunk。临时 24 小时保留使用 recursive pin，覆盖复用的强依赖。
- NamedStore 增加 File/ChunkList/Dir 本地提取；NFSP `commit_object` 与独立 FileBrowser 的对象提交负责目标 ACL、冲突、实体路径和任务自有 staging 清理。重试同一内容可回收已成功的结果；多根目录批次不宣称原子提交。
- MessageHub、HomeStation 以就绪对象作为附件/发布引用，目录可浏览和下载，读端支持 ChunkList，业务保留递归依赖。HomeStation 删除原 Blob PUT 上传接口；媒体类型保留业务约束。
- AIWorkspace 数据树和画布导入映射为 folder/AssetRef，继续使用 workspace ObjectStore ID，NDM ID 只用于来源和提取。每文件 32 MiB 限制保留。旧格式 staging 绑定 workspace/主体，失败可重试，成功结果跨重启恢复并按期清理；ZIP、图片替换、draw.io、快照与离线复制单独回归。
- AI Center 文件字段改为 SDK 就绪的 named_object；AICC 用原 token 验权，递归 TTL 保护同步/流式/native 异步请求，终态释放或有界到期。PIKG 本轮完成真实 SDK → staging 包校验 → inspect；整体安装计划/提交 UI 尚未连接真实服务，因此明确禁用该提交，不计作完整安装完成。
- 头像、图标、本地 Preview、Workflow JSON 和编辑器保存保留原业务边界，不强行增加目录语义。

### 7.2 已运行验证

| 范围 | 已获得的证据 |
| --- | --- |
| WebSDK / sys_test | SDK 134 项（NDM 52 + NDN 82）通过，含 Rust canonical QCID golden vectors；类型检查与构建通过。sys_test check/build 通过。 |
| NDM / NFSP | 真实 HTTP + 测试签名普通用户/RBAC；9 个服务测试通过，覆盖未登录/跨用户拒绝、TUS offset/续传/终止/过期、canonical ID/依赖完整性、零字节、QCID sampled bytes、目录与 ChunkList 提取、目标权限及重复提交。canonical AppInstance 隔离回归通过；NDM DV 已扩展 SDK 目录/重传/字节回读，Deno check 通过，未对旧部署执行新增协议。 |
| SDK → NDM | Chromium 和 Node 对新 NFS 实际上传/回读 0 B、48 KiB、33 MiB、中文根/空目录/嵌套同名文件。QCID 命中复验：48 KiB 与 33 MiB 均只采样读取 12288 B，内容上传 0 B；不再把“0 上传”误当成“未全量重读”。 |
| Desktop Files | Chromium → 独立 NFS/NDM 实际目录提交与字节回读通过；覆盖目录 picker、同名异路径、零字节及 mkdir/rename/search/delete 回归。 |
| AIWorkspace | Rust process 4 项、server guest 1 项、store packages 7 项、store guest 4 项通过；包含 workspace 绑定、其他用户拒绝、失败重试、重启重放、workspace 提交后 upload receipt 丢失恢复、历史重试不撤销较新 restore、目录替换中断恢复和权限保留。Chromium 6 项通过：数据树/画布目录层级与字节/重新打开，图片嵌入/链接，快照共享与 package import，draw.io 多页图片，ZIP new/restore，快照离线冷启动。 |
| HomeStation / MessageHub | HomeStation 54 项通过；msg_center 179 项通过、1 项需 Telegram 凭据的外网测试忽略。覆盖 ChunkList/目录、读授权、幂等发布、retention、已知 ID 拒绝且不创建消息/hold、原用户授权导入和可读消息复用。另 4 项 UI 回归通过，含目录枚举失败路径、目录消息/原始媒体、HomeStation 文件下载。 |
| AI Center / PIKG | AICC 资源 21、执行 53、原 token 鉴权 1；Playground 18 项通过。PIKG staging 8、鉴权 1、UI 2 项通过；验证真实大包/ChunkList 和服务端 digest，UI 明确阻止尚未接通的安装提交。 |
| Desktop 构建 | 最终 `pnpm check` / `pnpm build` 通过；保留现有 bundle 大小提示，无编译失败。 |
| 独立 FileBrowser | 独立 pnpm typecheck/build 与 `go test ./http` 通过，含本地对象提取/权限/重试测试；不以 Desktop 结果代替。 |

可重复的数据面联调入口：仓库根目录 `NFS_SERVER_BIN=/path/to/nfs_server node test/nfs_server_test/sdk_import_http.mjs`；Desktop 目录 `FB_NFSP_AUTO=1 pnpm exec playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts`。AIWorkspace 使用既有 `playwright.aiworkspace.config.ts`，现在同时启动临时 NDM。

本地跨仓库 Rust 验证通过命令行 Cargo patch 指向同级 `cyfs-ndn` 的 `named_store` / `ndn-lib`，未把机器绝对路径写入依赖 manifest。Desktop 使用同级构建后的 WebSDK。协议依赖必须一起交付，不能只更新单个前端。

### 7.3 尚未完成的部署验收

已只读检查现有 Zone 处于 Activated/Running；没有向该 Zone 安装或重启本轮修改。上述浏览器服务联调使用临时数据目录和随机本地端口；NDM 端到端夹具的 AuthDisabled 与服务单测的测试签名/RBAC 分别覆盖数据面和权限逻辑，不能替代 verify-hub、网关、AppInstance 与容器 SSO 的完整部署链路。

尚未验证更新后真实 Zone 的普通用户、非 owner AppInstance、跨应用秒传、移动设备目录能力，以及外部 AI provider 的实际异步请求/重启恢复、系统原生保存文件夹对话框。PIKG 完整安装提交仍是后续独立工作，本轮只验收真实包导入/检查。保留这些部署验收为未勾选项，不把 mock 或本地 fixture 通过写成线上验收完成。失败/取消后的浏览器 File 句柄仍需要重新选择，不声称页面刷新后自动续传。
