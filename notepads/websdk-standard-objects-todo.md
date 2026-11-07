# WebSDK 标准对象实现收敛 TODO

> 状态：未开始，第 2 节的决定已 review，可以交给 code agent。现状核实于 2026-11-06（buckyos `070d73afb`、buckyos-websdk `b25937d`、cyfs-ndn `eafd667`），下文行号以这些提交为准。
>
> 背景：2026-09-30 实现 MsgObject v2（cymsg breaking change）时发现，TypeScript 侧没有统一的标准对象实现。Desktop 手写了一份 `msgobj.ts` 镜像，WebSDK 的 `MsgObject` 只是 `Record<string, unknown>`。
>
> 目标：CYFS 标准对象的 TypeScript 实现统一放在 buckyos-websdk，前端只从 `buckyos` 包引用，不再各自维护协议镜像。同时修复 FileObject 常用 meta 缺失的 bug：构造 FileObject 时就写入常用 meta，不再放在对象之外。
>
> 协议权威：`cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md`（§5.2 JWT、§16 MsgObject、§17 ReceiptObj），参考实现是 `cyfs-ndn/src/ndn-lib`。Rust 与其它来源冲突时以 Rust 为准。

## 1. 现状

下表路径中，SDK 指 `buckyos-websdk/src/`，Desktop 指 `buckyos/src/frame/desktop/src/app/messagehub/`。

| 对象 / 能力 | Rust | SDK | 其它位置 |
| --- | --- | --- | --- |
| ObjId / ChunkId、canonical JSON、ObjId 计算 | 有 | 有 | — |
| `cyfile`、`cyinc`、`cyrel`、`cydir`、`cymap`、`clist` | 有 | 有 | — |
| `cypath` PathObject | `fileobj.rs:69`：`path, iat, target, exp, host?` | **已落后**：仍是 `uptime`，没有 `host`（`ndn_types.ts:1043`）。Rust 在 2026-04-19（`a56decf`）改的字段 | SDK 之外没有调用方 |
| `cymsg` MsgObject v2 | `msgobj.rs` | 只有常量；`msg_center_client.ts:31` 是 `JsonObject`，也没有从包入口导出 | Desktop `protocol/msgobj.ts`（610 行），被 src 48 个、tests 7 个文件引用 |
| `cyrece` ReceiptObj | `msgobj.rs:653` | 没有；`OBJ_TYPE_MSG_RECE = 'cymsgr'` 已过时（`ndn_types.ts:60`），无其它引用 | — |
| `cyact` ActionObject | `action_obj.rs` | 只有常量；`repo_client.ts:32` 的 `RepoActionProof` 是 `Record<string, unknown>`，而 Rust 里它是 `ActionObject` 的别名 | — |
| `pkg` PackageMeta | `package-lib/src/meta.rs` | 只有常量；`provision.ts:899,928` 有归一化和 ObjId 计算，但在 Node 专用模块里 | CLI `pikg_protocol.ts` 有手写字段校验 |
| JWT 形式的 NamedObject（§5.2） | `object.rs:400,433` | `loadNamedObjectFromObjStr` 抛 `Unsupported`（`ndn_types.ts:852`），`SimpleMapItem` 的 JWT 分支同样（`:1215`）。`namelib.ts` 已有 `decodeJwtClaimWithoutVerify`、`decodeJwtHeaderWithoutVerify`、`verifyJwtEdDSA`、`signJwtEdDSA`，且只依赖 `./types`，可直接复用 | Desktop `guest/model.ts:73` 手写了 `decodeNamedObject` |
| MsgObject 校验与 JWT（§16.5） | `MsgObject::validate`、`from_json_value_checked`、`decode_msg_object_jwt`、`verify_msg_object_jwt` | 没有 | Desktop 有 `isValidMsgSessionId`、`randomMsgNonce` |
| dispatch 编码 | `cyfs_http.rs:116-175` | 没有 | Desktop `decodeNamedObject` 靠 `contentType.includes('jwt')` 判断 |
| System Notify 类型 | `buckyos-api/src/msg_notify.rs` | 已有并已导出（`msg_center_client.ts:601` 起，`task_mgr_client.ts:537` 起） | Desktop `protocol/msgobj.ts:392-534` 重复定义 |
| FileObject 常用 meta | `fileobj.rs:23` 是 flatten 的 `meta`；《CYFS 标准对象》§10 的示例用 `mime` | **有 bug**：`ndm.importFiles` 构造时不写任何 meta（`ndm_client.ts:1015,1036,1088,1103,1117`） | HomeStation 把展示字段存在本地表 `import_metadata`，不随内容分发。`mime` 和 `mime_type` 混用，有的读取方只认其中一个，见 3.6 |

其它事实：

- **fixture 不能当作实现完成的证据。** `tests/ndn_object.json` 的 `cypath`、`cymsgr` 都是旧对象。`cyact`、`cymsg`、`cymsgr` 没有 TS 解码器，只校验原始 JSON 的哈希，所以现在 jest 全过。
- **`MsgReceiptObj` 不是 `cyrece`，不要改。** `msg_center_client.ts:191` 是 msg-center 读回执 RPC 的 DTO，对应 `buckyos-api/src/msg_center_client.rs:1212`，没有实现 `NamedObject`。
- **Desktop 不依赖 npm 发版。** 依赖是 `git+https://github.com/buckyos/buckyos-websdk#main`。`.pnpmfile.cjs` 调用 `src/tools/pnpm_local_sdk.cjs`，存在同级 SDK 时改用 `link:`（可用 `BUCKYOS_SDK_TOOL_SOURCE` 指定位置）。
- **SDK 的 `dist/` 和 `cli/dist/` 纳入了 Git。** Desktop 的类型来自 `dist/index.d.ts`，所以改完 SDK 源码必须先 `pnpm run build`，Desktop 才能看到新类型。

## 2. 已确认的决定

| # | 决定 | 做法 | 原因 |
| --- | --- | --- | --- |
| 1 | 新对象放哪 | 放 `ndn_types.ts`，经已有的 `ndn` 命名空间导出（`ndn.MsgObject`），不加顶层导出 | 三个入口都已 `export * as ndn`，Desktop 也已在用 `ndn.FileObject` |
| 2 | 新对象的写法 | `MsgObject`、`ReceiptObj`、`ActionObject`、`PackageMeta` 都用 interface 加函数，不用 `NamedObjectBase` class | 它们以普通 JSON 在 RPC 和 React state 里流动，Desktop 还会展开并附加 `ui_*` |
| 3 | `agent_task`、`agent_ask`、`voice` | 类型和读取函数留在 Desktop，SDK 的 `MsgObject` 只用索引签名承载 | 它们是 OpenDAN 与 MessageHub 之间的约定，不在《CYFS 标准对象》里 |
| 4 | Notify | 类型和常量改用 SDK 已有的；`messageNotify`、`parseHumanTaskPrompt` 和 `NOTIFY_CATEGORIES/LEVELS/STATUSES` 留在 Desktop | SDK 的 `notifyMessageData` 只做类型断言，Desktop 的版本会清洗不可信数据 |
| 5 | 验签 | 提供 `verifyMsgObjectJwt`，不提供 TS 侧签名 | `namelib.verifyJwtEdDSA` 已有，不加依赖，还能完整移植 JWT 用例。Desktop 暂无调用点 |
| 6 | fixture 范围 | 覆盖 `cyfile cypath cyinc cyrel cyact cymsg cyrece pkg` 共 8 种 | 这是 Rust 回环测试实际输出的范围；`cydir`、`cymap` 不在其中，保留现有 TS 用例 |
| 7 | HomeStation 的 `FileObject`（`src/frame/desktop/src/app/homestation/protocol/feed.ts:74`） | 删除这个投影类型，后端直接返回标准 `cyfile` JSON，Desktop 用 `ndn.FileObjectJson` | 常用 meta 写进对象之后，就不再需要投影 |
| 8 | 校验调用点 | 这次不在 Desktop 新增校验调用，只迁移实现 | 保持收发行为不变；接入校验可单独提 |
| 9 | 与 Rust 同名但取值不同的常量 | 这次一起改，见 3.1 | 只有 `ndn_types.ts` 内部在用，外部没有引用，改动很小 |
| 10 | 常用 meta 的范围 | `mime`、`width`、`height`、`duration_ms`，写进《CYFS 标准对象》§10 | 这是 UI 实际在用的字段，都能在构造时确定 |
| 11 | mime 的键名 | 统一为 `mime`，删除 `mime_type` 的读写，不做兼容 | §10 示例、fixture、`bucket_store`、nfs 快照、HomeStation 都用 `mime` |
| 12 | 在哪里探测 | 放进 SDK 的 `ndm.importFiles` | HomeStation、MessageHub、filebrowser、aiworkspace 的上传都走它；放在 Desktop 又只修了一处 |
| 13 | 已有对象 | 不迁移，缺字段就按缺失处理 | 开发阶段不做兼容 |

决定 7、10–13 的背景：展示字段不进 FileObject 是 bug。FileObject 这一层就是用来放 meta 的，构造时应该把常用 meta 都写进去。现在浏览器上传的展示字段只存在本用户的 `import_metadata`（`publish.rs:909`），不随内容分发，所以关注者拿不到 mime、尺寸和时长。架构文档（`doc/homestation/BuckyOS HomeStation 架构设计.md:371`）的写法是对的，协议文档（`doc/homestation/HomeStation 协议与实现.md:59`）要改。

影响：相同字节但 meta 不同的文件，FileObject 的 ObjId 不同，和 `name` 不同时一样。探测结果可能因浏览器而异，所以同一文件从不同浏览器上传也可能得到不同的 ObjId。chunk 层的去重不受影响。

## 3. 待办

建议按小节顺序做。3.1–3.5 在 buckyos-websdk，3.6–3.8 在 buckyos；cyfs-ndn 只有 3.6 的两处小改动和 3.8 的文档。

### 3.1 SDK：修正已落后的内容，重新生成 fixture

- [ ] `PathObject` 改为 `iat`，增加可选的 `host`（为空时省略），同步构造函数、`fromJSON`、`toJSON` 和 `tests/ndn_types_cases.ts:670,692,825-831`。
- [ ] `OBJ_TYPE_MSG_RECE` 改名为 `OBJ_TYPE_RECEIPT`，值为 `cyrece`，与 Rust 同名。
- [ ] 其它 `OBJ_TYPE_*` 常量按 `cyfs-ndn/src/ndn-lib/src/lib.rs` 对齐：

  | SDK 现在（`ndn_types.ts:67-78`） | 改为 |
  | --- | --- |
  | `OBJ_TYPE_OBJMAP = 'cymap-mtp'`、`OBJ_TYPE_OBJMAP_SIMPLE = 'cymap'` | `OBJ_TYPE_OBJMAP = 'cymap'` |
  | `OBJ_TYPE_LIST = 'cylist-mtree'`、`OBJ_TYPE_LIST_SIMPLE = 'cylist'` | `OBJ_TYPE_LIST = 'cylist'` |
  | `OBJ_TYPE_CHUNK_LIST = 'cl'`、`OBJ_TYPE_CHUNK_LIST_SIMPLE = 'clist'` | `OBJ_TYPE_CHUNK_LIST = 'clist'` |
  | `OBJ_TYPE_TRIE`、`OBJ_TYPE_TRIE_SIMPLE`、`OBJ_TYPE_CHUNK_LIST_FIX_SIZE`、`OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE` | 删除，Rust 已注释掉 |

  同步 `isChunkList`、`isContainer`、`isBigContainer`，判断规则同 `object.rs:93,116,127`，并加用例。现在 `isBigContainer` 对 `clist`、`cymap`、`cylist` 返回 `false`，而 Rust 返回 `true`；TS 侧目前没有调用方，所以没有实际影响。需要改的引用只有 `ndn_types.ts:563,582-604,1568,1572,1732` 和 `tests/ndn_types_cases.ts:46,180,1005`；SDK 的 `src`、`tests`、`cli/modules` 和 Desktop、`src/tools`、`test`、`src/apps` 都已搜过，没有其它引用。
- [ ] 重新生成 `tests/ndn_object.json`，并删除 `tests/ndn_types_cases.ts` 中关于 `cymsgr` 的注释。在 `cyfs-ndn/src/` 运行：

  ```bash
  # 打印 7 项的数组（输出夹在测试日志里，需要提取）
  cargo test -p ndn-lib --lib test_non_chunk_named_object_file_roundtrip -- --nocapture
  # 打印单个对象，追加为第 8 项
  cargo test -p package-lib --lib test_package_meta_file_roundtrip -- --nocapture
  ```

  已实跑确认的预期结果：`cyfile`、`cyinc`、`cyrel`、`cyact`、`cymsg` 不变；`cypath` 变为 `cypath:b521d401…`；`cymsgr` 被 `cyrece:f7564f5b…` 取代；新增 `pkg:368aa598…`。

### 3.2 SDK：MsgObject v2

- [ ] 在 `ndn_types.ts` 增加类型：`MsgObject`、`MsgObjKind`、`TopicThread`、`MsgRelation`、`MsgRelType`（保留未知值）、`MsgMentions`、`MsgContent`、`MsgContentFormat`、`MachineContent`、`CanonValue`、`RefItem`、`RefTarget`、`RefRole`。可以从 Desktop `protocol/msgobj.ts:23-149,211-239` 搬过来，已对照 Rust 确认字段一致。`meta` 是 flatten 字段，用索引签名表示。`created_at_ms` 沿用 Desktop 的必填写法，避免 Desktop 到处判空；校验入口仍接受缺失。
- [ ] 增加校验：`validateMsgObject`、`msgObjectFromJsonChecked`（对应 `from_json_value_checked`）、`isValidMsgSessionId`、`MSG_OBJECT_RESERVED_KEYS`、`MSG_SESSION_ID_MAX_CHARS`、`MSG_REACTION_KEY_MAX_BYTES`、`randomMsgNonce`。规则见第 4 节。
- [ ] `msg_center_client.ts` 的 `MsgObject` 改为 `ndn_types.ts` 的类型，并去掉 `msgMachineData` 等处多余的 `as JsonObject`。
- [ ] 移植测试，对应关系见第 4 节末尾。

### 3.3 SDK：ReceiptObj、ActionObject、PackageMeta

- [ ] 增加 `ReceiptObj`、`ReceiptStatus`（`accepted | rejected | quarantined`）。
- [ ] 增加 `ActionObject`、`ACTION_TYPE_*` 常量。`repo_client.ts` 的 `RepoActionProof` 改为 `ActionObject`，并更新该处“保持 opaque”的注释。
- [ ] 增加 `FileObjectJson` interface，表示 `cyfile` 的普通 JSON：`name?`、`size?`、`content?` 等基础字段，`mime?`、`width?`、`height?`、`duration_ms?`，再加索引签名。现有的 `FileObject` class 不动。
- [ ] 增加 `PackageMeta`（`FileObjectJson` 加 `version`、`version_tag?`、`deps?`）。把 `normalizePackageMetaForObjId`、`calcPkgMetaObjId` 移到 `ndn_types.ts`，让浏览器也能用，`provision.ts` 改为引用它们。`tests/provision.test.ts:403` 必须继续通过。
- [ ] 把 `cyact`、`cymsg`、`cyrece`、`pkg` 注册进 `tests/ndn_types_cases.ts:172` 的 `TEST_NAMED_OBJECT_DECODERS`，让它们走“解码再编码”的路径，而不是只校验哈希。

### 3.4 SDK：JWT 与 dispatch

- [ ] `loadNamedObjectFromObjStr` 按 Rust 改写（`object.rs:433`）：去掉开头空白后以 `{` 或 `[` 开头按 JSON 解析，否则按 JWT 取 claims。现在的 `indexOf('{') >= 0` 判断不对。
- [ ] 增加 `buildNamedObjectByJwt`、`verifyNamedObjectFromJwt`，并实现 `SimpleMapItem` 的 `objectJwt` 分支。ObjId 只由 claims 计算。
- [ ] 增加 `decodeMsgObjectJwt`、`verifyMsgObjectJwt`、`msgJwtKidDid`。header 必须 `alg = EdDSA` 且 `kid` 非空，claims 走 `msgObjectFromJsonChecked`。
- [ ] 增加 `CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON`、`CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT`、`namedObjectEncodingFromContentType`、`isCyfsNamedObjectContentType`。判断规则同 `cyfs_http.rs:167-175`：去掉 `;` 后的参数，大小写不敏感。

### 3.5 SDK：上传时写入常用 meta

- [ ] 把探测能力移进 SDK。`sniffMagic`、`stripMediaTypeParams`、`heifImageSize` 从 Desktop `src/frame/desktop/src/components/preview/mediaTypes.ts` 移到 `src/content_mime.ts`，经 `buckyos/content` 导出（`extensionOf` 已是这个模式），Desktop 改为引用。
- [ ] 增加 `probeFileMeta(file)`，逻辑来自 Desktop `app/homestation/api/fileKind.ts` 的 `sniffFile` 和 `app/homestation/api/store.ts:76,92` 的 `loadMedia`、`uploadMeta`。规则：
  - `mime`：媒体文件以 magic bytes 为准，其它文件用声明的类型或扩展名，不带 `charset` 等参数。判断不出就省略，不写 `application/octet-stream`。
  - `width`、`height`：图片和视频都写（现有代码只写了图片）。HEIC 解码失败时用 `heifImageSize`。
  - `duration_ms`：音频和视频，整数毫秒。
  - 没有 DOM 的环境（Node、Deno、Worker）只写 `mime`。
  - 探测失败或超时只省略对应字段，不能让导入失败。现有的 `loadMedia` 没有超时，要加上。
- [ ] `importFiles` 在 QCID 查找之前探测，并把结果传给 `materializeFile`、`materializeFileViaWorker`、`buildFileObjectFromContentId`，让所有 `new NdnFileObject(...)` 都带上 meta。
- [ ] **改掉 `ndm_client.ts:1513` 的重建逻辑。** 它在 QCID 命中时按 `name/size/content` 重建对象来取 `obj_data`，不带 meta 就会和 `objectId` 对不上。
- [ ] `ImportedFileObject.mimeType` 改取探测结果，不再用 `file.type`。
- [ ] `importFiles` 增加可选的 `fileMeta` 参数接收调用方已有的探测结果，这样 HomeStation 校验附件类型时不用探测两次。
- [ ] 测一下批量导入文件夹时的探测开销，必要时限制并发。
- [ ] 更新写死 ObjId 和 mimeType 的用例：`tests/ndm_client.test.ts:211`、`tests/ndm_client_cases.ts:147,178,577,578,841,854`。jest 没有解码能力，所以在 `tests/browser/real-browser/ndm_client.spec.js` 加真实浏览器用例，验证图片尺寸和音视频时长。
- [ ] `pnpm run build`，让 `dist/` 和 `cli/dist/` 包含 3.1–3.5 的改动。

### 3.6 服务端：统一键名，HomeStation 改用 cyfile

本节路径相对 buckyos 的 `src/`，`bucket_store` 在 cyfs-ndn。

- [ ] 写入 FileObject 时改用 `mime`。构造时已经知道尺寸或时长的地方也要写上（如 `frame/aicc/src/video/mod.rs:954`、`kernel/workflow/src/functions/media/`），但不要为此新增解码。

  | 位置 | 现在 | 改为 |
  | --- | --- | --- |
  | `kernel/workflow/src/io.rs:56` | `mime_type` | `mime` |
  | `frame/agent_tool/src/llm_understand_media/pipeline.rs:385` | `mime_type` | `mime` |
  | `frame/aicc/src/resource/mod.rs:38,887,2243` | `mime_type` | `mime` |
  | `frame/msg_center/src/tg_tunnel.rs:691` | `mime_type` | `mime` |
  | cyfs-ndn `src/bucket_store/src/s3.rs:290`、`content.rs:43` | `mime`，值可能带参数 | 去掉 `charset` 等参数 |

- [ ] 读取 FileObject 时只认 `mime`：`frame/agent_tool/src/read_tool.rs:118`、`frame/agent_tool/src/llm_understand_media/pipeline.rs:431`、`frame/msg_center/src/object_access.rs:411`、`frame/aicc/src/resource/mod.rs:1607`。
- [ ] 以下的 `mime_type` 不是 FileObject 字段，**不要改**：`frame/aicc/src/protocol/` 下的第三方协议（`gemini.rs`、`fal_queue.rs`）、`frame/msg_center/src/tg_tunnel.rs:387,451` 的附件 DTO、SDK `ndm_client.ts:1755` 的 tus 元数据。
- [ ] 以下对象**不要加 meta**：`frame/control_panel/src/buckets.rs:151`、`frame/repo_service/src/service.rs:183`、SDK `cli/modules/app.ts:1111`。这些是包内容，CLI 和服务端都按 `name/size/content` 算出同一个 ObjId。
- [ ] 给 HomeStation 加跨用户用例：在 `frame/homestation/tests/` 用 `Net::new(&["carol", "bob"], …)`，carol 发布带 `mime`、`width`、`height` 的 `cyfile` 图片，bob 看到的 `media[0].file` 要有这些字段，`/content` 的 Content-Type 要正确。bug 的根因是 SDK 不写 meta，由 3.5 的用例覆盖；这个用例防止 HomeStation 的读取路径再丢字段。
- [ ] `frame/homestation/src/projection.rs`：删除 `file_view`，`local_file` 直接返回对象 body；`resources.rs:409` 同样处理。
- [ ] 删除 `import_metadata`：表定义 `kernel/buckyos-api/src/homestation_client.rs:137`，以及 `frame/homestation/src/` 下的 `objects.rs:509`、`publish.rs:904-914,929-931,1142-1144`、`projection.rs:126`、`http.rs:360-361`。媒体类型校验和 Content-Type 改读对象的 `mime`。
- [ ] `object.register` 去掉 `metadata` 参数（`frame/homestation/src/api.rs:154`、`publish.rs:876`）。
- [ ] 更新 `frame/homestation/tests/reading.rs:11,66,130`、`sources.rs:109`、`object_import.rs:70,81,198`。

### 3.7 Desktop 及其它消费方

先确认 Desktop 能解析到新版 SDK：本地有同级仓库时走 `link:`，否则要等 SDK 合入 main。

本节的 `protocol/`、`guest/` 相对 Desktop 的 messagehub 目录，`tests/` 相对 `src/frame/desktop/`，其余相对 buckyos 仓库根目录。

- [ ] `protocol/msgobj.ts` 按下表收敛，并改写文件头（现在还指向旧的 Mac 路径）。其它文件的协议类型直接从 `buckyos` 引入，不留转发层。

  | 内容 | 处理 |
  | --- | --- |
  | 3.2 列出的协议类型、`MSG_SESSION_ID_MAX_CHARS`、`isValidMsgSessionId`、`randomMsgNonce` | 删除，改用 `ndn.*` |
  | `DID` | 改用 `buckyos` 顶层导出的 `DID`（SDK `types.ts:1`） |
  | `ObjId`、`Uri`、`JsonValue` | 确认无外部引用后删除，内部用 `string` |
  | Notify 类型和常量：`NOTIFY_INTENT`、`NOTIFY_SESSION_PREFIX`、`NOTIFY_CHOICE_CONTROL_ID`、`NOTIFY_TEXT_CONTROL_ID`、`Notify{Kind,Category,Level,Status,Conversation,Source,MessageData}`、`HumanPrompt{Option,Control}`、`HumanTaskPrompt` | 删除，改用 SDK 已有的 |
  | `notifySessionId` | 换成 `notifyCategorySessionId` |
  | `notifySessionCategory` | 换成 `notifyCategoryFromSessionId`。注意它不匹配时返回 `null` 而不是 `undefined`，调用方要逐个检查 |
  | `MessageObject` | 保留，定义为 `ndn.MsgObject & { agent_task?: AgentTaskRef; agent_ask?: AgentAsk; voice?: VoiceNote }` |
  | `AgentTaskRef`、`AgentAsk*`、`AGENT_ASK_ANSWER_INTENT`、`VoiceNote`、`VOICE_WAVEFORM_*`、`messageObjId`、`messageAgentTaskId`、`messageAgentAsk`、`messageVoice`、`messageAgentAskAnswer`、`agentAskTarget`、`buildAgentAskAnswer`、`agentAskQuestionText` | 保留 |
  | `messageNotify`、`parseHumanTaskPrompt`、`NOTIFY_CATEGORIES`、`NOTIFY_LEVELS`、`NOTIFY_STATUSES` | 保留 |
  | `MessageDeliveryStatus`、`ConversationStatusType`、`getMessage*`、`isStatusMessageObject` | 保留。`ui_*` 字段经索引签名访问，不需要声明 |

- [ ] `guest/model.ts:73` 的 `decodeNamedObject` 删除，`guest/api.ts:84` 改用 `ndn.loadNamedObjectFromObjStr`。
- [ ] 同步 `tests/datamodel/messagehub-*.test.ts` 的 import（如 `messagehub-session.test.ts:4`）和 `messagehub-guest.test.ts:53-54` 的断言。
- [ ] 删除 `src/frame/desktop/pnpm-workspace.yaml` 中已无意义的 `minimumReleaseAgeExclude: buckyos@0.7.116`。
- [ ] `test/jarvis_media_dv/jarvis_media_dv.ts:66-85` 的本地 `RefItem`、`MsgObject` 改用 SDK 类型。其中 `source?` 不是 v2 字段，改之前确认是否还在用。
- [ ] 检查其它 SDK 消费方是否因类型收紧而需要调整：`src/tools/buckyos-agent/lib/ask.ts`、`test/test_msg_center/`、`test/test_sys_test/support_group_dv.ts`、`test/jarvis_media_dv/scenarios.ts`。
- [ ] HomeStation 改用标准 `cyfile`。在 `src/frame/desktop/src/app/homestation/` 下：删除 `protocol/feed.ts:74` 的 `FileObject`，`datamodel/types.ts`、`media.ts`、`MediaThumb.tsx`、`mock/` 改用 `ndn.FileObjectJson`；读取处从 `file.meta.x` 改成 `file.x`（`media.ts:55,71`、`card/parts.tsx:198,228`、`detail/ItemDetail.tsx:81`、`mock/media.ts:15-16`、`mock/data.ts:252-254`、`mock/store.ts:1058`）。
- [ ] 同一目录下，`api/store.ts:76,92` 的 `loadMedia`、`uploadMeta` 和 `api/fileKind.ts` 的 `sniffFile` 改用 SDK 的 `probeFileMeta`，保留附件类型校验；`api/transport.ts:17,98,110,114` 精简 `UploadMeta`，`object.register` 不再传 `metadata`。
- [ ] `src/frame/desktop/src/components/preview/ndmSource.ts:95`、`src/frame/desktop/src/app/messagehub/api/objects.ts:79` 改读 `mime`。

已扫描过 `src/`、`test/` 下的其它前端：除上述位置外，没有发现自定义的标准对象类型（只按名称匹配）。filebrowser、OpenDAN WebUI、node_active、control_panel 都没有命中。

### 3.8 文档

- [ ] `cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md` §10 增加常用 meta 键的定义：`mime`（不带参数）、`width`、`height`、`duration_ms`。
- [ ] `doc/homestation/HomeStation 协议与实现.md:59` 改为常用 meta 写进 `cyfile`，并同步 `object.register` 的描述；`doc/homestation/HomeStation Durable Data Schema.md:74,124` 删除 `import_metadata`。
- [ ] 更新 `doc/sdk/扩展self-host-group.md:399`，MsgObject 等类型的出处改为 SDK。
- [ ] 检查 `product/message_hub/MessageHub_Current_UI_Model_Data.md:218` 的描述是否仍然成立。

## 4. MsgObject 校验的移植规则

`validateMsgObject` 对应 `msgobj.rs:483`：

- `to_session` 存在时取值合法，且 `to` 只有一个目标。
- `mentions` 存在时不能为空（`dids` 为空且 `all` 不为真）。
- `relates_to.target` 的 ObjType 必须是 `cymsg`。
- `reaction` 必须带 `key`，长度 1–64 字节；已知的其它 `rel` 不能带 `key`；未知 `rel` 不检查 `key`。
- 不能出现保留键。

容易写错的地方：

- **`key` 按 UTF-8 字节计数**（用 `TextEncoder`），`to_session` 按 code point 计数（200 个）。
- **`trim()` 有差异。** JS 的 `trim()` 会去掉 U+FEFF，Rust 不会。Desktop 现有写法对 U+FEFF 开头的值判断与 Rust 相反，迁移时要修正。
- **`meta` 是扁平的。** TS 对象上没有独立的 `meta`，声明字段不可能混进去，所以保留键检查实际只会命中 `proof`。

`msgObjectFromJsonChecked` 对应 `msgobj.rs:545`：用原始值算 ObjId，解析并校验，再把值按 serde 规则归一化后重算 ObjId，两者不同就拒绝。归一化和解析要覆盖：

- **省略默认值：** 值为 `null` 的可选字段、空 `thread`、`created_at_ms: 0`、`content.content: ""`、`refs: []`、`machine.data: {}`、`mentions.dids: []`、`mentions.all: false`。比如 `workspace: null` 必须被拒绝。
- **`format` 别名：** `image/jpg`、`image/svg`、`video/avi`、`audio/mp3`、`audio/x-wav` 在 Rust 反序列化时会转成标准值，所以这些输入因不是 canonical 形式而被拒绝。
- **封闭枚举：** `kind`、`RefRole`、`RefTarget.type` 遇到未知值就拒绝。`rel`、`format` 是开放的，保留原值。
- **必填字段：** `from`、`to`、`kind`、`content`。
- **DID：** 必须是 `did:method:id` 形式。Rust 虽然接受主机名，但转换后不再 canonical，结果同样是拒绝。
- **ObjId 字段：** `thread.reply_to`、`relates_to.target`、`RefTarget.obj_id` 都要能解析。
- **大整数：** Rust 的 `nonce`、`created_at_ms`、`expires_at_ms` 是 u64，而 `JSON.parse` 超过 2^53 会丢精度并算出错误的 ObjId。这些字段不是非负安全整数时要报错。当前 Rust 发送方都在范围内（`lib_opendan/src/protocol/input.rs:795` 是 u32，`msg_center/src/group_service.rs:1393` 是序号）。这与 Rust 有意不同，只影响超出范围的值。

测试对应 `msgobj.rs` 的 `test_msg_v2_*`（第 1400 行起），外加 `test_msg_checked_rejects_canonicalization_errors`。以下几处不能照搬：

| Rust 用例 | TS 处理 |
| --- | --- |
| `test_msg_v2_proof_is_reserved` | 只移植 `proof` 部分。`meta.insert("to_session", …)` 在扁平结构下无法表达 |
| `test_msg_v2_jwt_form` | 用 `namelib.signJwtEdDSA` 和 Rust 同一把测试密钥（`msgobj.rs:1545-1546`）现签。“无效对象不签名”一段没有对应，因为 TS 不提供签名 |
| `test_msg_checked_rejects_canonicalization_errors` | `JSON.parse('1e400')` 得到 `Infinity`，应由 `canonicalizeJson` 拒绝 |

## 5. 约束

- 修改《CYFS 标准对象》时，同一次改动同步更新 SDK 的类型、校验和 fixture。
- canonical JSON 必须与 Rust `serde_jcs` 一致，否则 ObjId 不同。
- 反序列化保持宽松，校验由入口显式调用，与 Rust 一致，保证旧数据能读出。
- 不做向下兼容，不加新依赖（JWT 和验签都复用 `namelib`）。
- 跨仓顺序：SDK 改动（含 `dist/`、`cli/dist/`）先进入 buckyos-websdk 的 main，再合 buckyos 的改动。没有同级 SDK 的环境从 GitHub main 取 SDK，顺序反了 Desktop 的 `tsc` 会失败。
- cyfs-ndn 只改 `bucket_store` 的两处 mime 写入和文档，没有接口变化，与其它仓库没有合入顺序要求。
- 协议字段和类型的改动都不加兼容分支：`mime_type` 回退读取要删掉，HomeStation 的 `file.meta` 结构也不保留。
- `src/rootfs/libexec/buckyos-tool/` 是构建产物，不要手改。

## 6. 验证

基线是 2026-11-06 改动前实跑的结果，用来区分已有失败和新引入的失败。

| 位置 | 命令 | 基线 |
| --- | --- | --- |
| `buckyos-websdk/` | `npx jest tests/ndn_types.test.ts tests/msg_center_client.test.ts tests/provision.test.ts tests/namelib.test.ts tests/types.test.ts` | 5 suites，227 通过 |
| `buckyos-websdk/` | `npx jest tests/ndm_client.test.ts tests/ndm_picker.test.ts` | 2 suites，59 通过 |
| `buckyos-websdk/` | `pnpm test` | 未跑 |
| `buckyos-websdk/` | `pnpm run build` | 未跑 |
| `buckyos-websdk/` | `pnpm run test:browser:real`（需要真实环境） | 未跑 |
| `src/` | `cargo test -p homestation --test reading --test object_import --test sources -- --test-threads=1` | 12 通过 |
| `src/` | `cargo test -p workflow -p agent_tool -p aicc -p msg_center -- --test-threads=1` | 未跑 |
| `src/frame/desktop/` | `pnpm check` | 通过 |
| `src/frame/desktop/` | `deno test --no-check --config tests/datamodel/messagehub.deno.json --allow-all tests/datamodel/messagehub-*.test.ts` | 92 通过，7 失败 |
| `src/frame/desktop/` | `pnpm exec playwright test --config playwright.homestation.config.ts` | 未跑 |
| `src/frame/desktop/` | `pnpm lint`、`pnpm build` | 未跑 |

注意：

- **7 个基线失败都在 `messagehub-guest.test.ts`，单独运行也复现，原因未查。** 其余 messagehub 用例 90 个全过。
- **这些失败会挡住对 `decodeNamedObject` 的验证。** 第一个用例在第 46 行就失败，走不到第 53–54 行的 JWT 断言。替换后要用别的用例覆盖，比如在 SDK 测 `loadNamedObjectFromObjStr`。
- **`verifyMsgObjectJwt` 的浏览器分支 jest 覆盖不到。** `namelib` 在没有 Node 运行时才走 WebCrypto，这部分如未实测要如实说明。
- **提交说明要按 `AGENTS.md` 的 cmd_skills 联动表**写明受影响的手册，或写明无需修改的依据。手册和 `doc/sdk/` 里已搜过没有 `mime_type`，但 AICC 产物的 meta 变了，要核对 `aigc_*`。
- **改 `agent_tool` 后要重建 paios 镜像**（`./build_aios --local-test`），否则容器里跑的还是旧版。
- **已有对象不迁移**（决定 13）。测试 HomeStation 附件时要新上传，旧附件没有这些字段，开发 Zone 需要干净数据时用 `start.py --all --wipe-data`。
- 未执行的检查必须如实说明。依赖 Zone 的 DV（`test/jarvis_media_dv` 等）跑不了时标为未验证。

## 7. 本次不做

- `validate_cyfs_dispatch_body` 的 TS 移植：这是服务端入口逻辑，TS 侧没有消费方。
- CLI `pikg_protocol.ts` 的手写校验：属于发布校验逻辑，不改。
- `cyfs-ndn/doc/CYFS Protocol/Content Network.md:473,587` 仍写着 `cymsgr`：这是 cyfs-ndn 的文档问题，另行修正。

## 8. 验收

- `ndn` 命名空间提供第 3 节列出的所有类型和函数，浏览器入口和 Node 入口都可用。
- SDK 的 `PathObject`、`OBJ_TYPE_*` 常量和 `isChunkList`、`isContainer`、`isBigContainer` 与 Rust 一致。
- `tests/ndn_object.json` 含 8 种对象，全部走解码再编码的路径，jest 按 fixture 重算 ObjId 全部通过。
- 第 4 节列出的 Rust 用例都有对应的 TS 用例，结果一致。
- Desktop 中不再有协议对象的类型定义，只保留决定 3、4 和第 3.7 节表格列出的内容。
- `ndm.importFiles` 产出的 `cyfile` 带 `mime`；在浏览器里，图片和视频还带 `width`、`height`，音视频还带 `duration_ms`。QCID 命中时的 ObjId 和 `obj_data` 也一致。
- 关注者看到的媒体带这些字段，`/content` 的 Content-Type 正确，由 3.6 的跨用户用例证明。
- HomeStation 接口的 `file` 字段是标准 `cyfile` JSON，代码里不再有 `import_metadata` 和 `file_view`。
- `mime_type` 不再作为 FileObject 的键被读写（3.6 列出的非 FileObject 用法除外）。
- 第 6 节的命令没有新增失败：`pnpm check` 通过，messagehub 用例的失败不超出基线那 7 个。
