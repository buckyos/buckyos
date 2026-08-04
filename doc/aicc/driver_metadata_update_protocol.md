# AICC Driver Metadata 云更新协议

状态：beta 2.2 v1 基线

本文只定义按 provider-driver 拆分的 metadata JSON 的发布、下载和生效协议。OpenRouter 等聚合 Provider 的模型名解析不在范围内。

## 1. 设计目标

- 只下载新增或变化的 provider JSON。
- 一次发布以完整 manifest 为原子单位；运行时只能看到完整旧版本或完整新版本。
- 任意时刻断电、进程退出或网络中断后，旧的 LKGS 仍可用。
- 小文件始终整文件重下，不恢复下载进度，不持久化 `updating` 阶段。
- beta 2.2 是 v1 兼容基线；此后的不兼容变化必须提升 major version。

## 2. 发布路径

AICC settings 中显式启用更新源：

```json
{
  "driver_metadata_update": {
    "enabled": true,
    "source_url": "https://<publisher-zone-host>/aicc/driver-metadata/index.json",
    "interval_secs": 3600
  }
}
```

`source_url` 的 HTTPS host 是发布者信任锚，path 固定为 `/aicc/driver-metadata/index.json`。每个 canonical `source_url` 使用独立的本地水位和 activation namespace，不跨发布源比较 revision。settings 日志必须掩盖 URL userinfo。未配置、配置无效或 `enabled=false` 时停止轮询。客户端最多保留最近使用的四个 source namespace；重新启用仍在保留窗口内的 canonical source 时继续沿用原有 LKGS 和水位，超出窗口的最旧 namespace 会被整体回收。settings 暂时不可用是独立状态，只退避重试，不能按禁用处理或清理当前 LKGS。`interval_secs` 归一化到 60 秒至 1 天。

```text
/aicc/driver-metadata/index.json
/aicc/driver-metadata/v1/manifest-<revision_seq>.json
/aicc/driver-metadata/v1/providers/<provider_driver>-<revision_seq>.json
```

发布目录由 `NdnDirServer` 扫描、对象化并签发 PathObject。发布顺序必须是 provider 文件、manifest、index；index 最后更新。

PathObject 的签名、host、path 和 `exp` 由 NDN SDK 验证，AICC 不另设 TTL 上限。业务内容回滚也必须发布更高的 `revision_seq`，不得重新使用旧 revision。

## 3. Index

```json
{
  "format": "buckyos.aicc.driver-metadata-index",
  "index_version": 1,
  "index_revision": 0,
  "index_revision_seq": 42,
  "required_features": [],
  "tracks": [
    {
      "protocol_version": 1,
      "protocol_revision": 0,
      "revision_seq": 42,
      "required_features": [],
      "manifest": {
        "path": "v1/manifest-42.json",
        "obj_id": "<FileObject ObjId>"
      }
    }
  ]
}
```

- `index_version` 是稳定 index major；v1 客户端只接受 `1`。
- `index_revision_seq` 全局严格递增；同 revision 的 index 内容或 ObjId 不同是发布冲突。
- `tracks` 按 protocol major 唯一。客户端只选择自己支持且 `required_features` 全部已知的 track。
- track 的 `protocol_revision` 只允许增加具有缺省语义的可选字段；`revision_seq` 与该 track manifest 一致。
- `manifest.path` 必须是相对 index 目录的 canonical path，禁止 `..`、百分号编码、query、fragment 和绝对 URL；URL join 后仍必须位于 `/aicc/driver-metadata/` 目录内。

未来发布 protocol v2 时在同一 index 并列增加 v2 track，不能替换 v1 track；因此旧客户端仍可获得 v1 安全修复。

## 4. Manifest

```json
{
  "format": "buckyos.aicc.driver-metadata-manifest",
  "protocol_version": 1,
  "protocol_revision": 0,
  "revision_seq": 42,
  "required_features": [],
  "files": [
    {
      "provider_driver": "openai",
      "path": "v1/providers/openai-18.json",
      "schema_version": 2,
      "revision_seq": 18,
      "obj_id": "<FileObject ObjId>"
    }
  ],
  "tombstones": [
    {
      "provider_driver": "removed-provider",
      "revision_seq": 7
    }
  ]
}
```

- `files` 是该 revision 的完整 active provider 集合，`provider_driver` 唯一。
- 未变化文件的 `revision_seq` 和 `obj_id` 必须保持不变。
- 新增 provider 只增加一项；修改 provider 只提高该项 revision 并更换 ObjId。
- 删除必须从 `files` 移除并增加更高 revision 的 tombstone；无 tombstone 的缺失视为损坏 manifest。tombstone 集合是累积集合，不能在后续 manifest 中无故消失或降低 revision。
- track 和 manifest 的 `revision_seq` 必须相同，PathObject target 必须等于 track 中的 manifest ObjId。

## 5. Provider metadata

```json
{
  "format": "buckyos.aicc.provider-driver-metadata",
  "schema_version": 2,
  "schema_revision": 0,
  "provider_driver": "openai",
  "revision_seq": 18,
  "required_features": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "version_rules": []
}
```

文件内 `provider_driver`、`schema_version`、`revision_seq` 必须与 manifest 项一致。未知字段以及无效的 model id/pattern、variant、mount、token limit、成本和质量值均 fail-closed。`schema_revision` 可以增加具有明确缺省语义的可选字段；需要新解释能力的变化必须同时加入 `required_features`。不兼容结构变化提升 `schema_version`。

## 6. 严格下载

客户端使用 `CyfsNdnClient` 下载每个完整文件。每个响应都必须：

1. 存在且成功验证 `response.meta().path_object`，禁止退回只信任未签名 `cyfs-obj-id` header。
2. 通过 SDK 对 PathObject signer scope、host、path、exp 的验证。
3. manifest/provider 的 PathObject target 与上级声明的 FileObject ObjId 相同。
4. 通过 SDK 完成 FileObject/Chunk 链和内容校验。

NDN SDK 是下载时文件内容正确性的唯一协议校验层，AICC 不重复实现 ObjId 或 ChunkList 校验。首次验证成功落盘时，AICC 记录裸内容 SHA-256；后续缓存复用重新计算该摘要，以发现落盘后的静默损坏。为在下载前执行容量限制，AICC 只读取 SDK 已验证的 ChunkId 长度，或已验证 parent 中 FileObject/ChunkList 的长度声明；没有可信长度时 fail-closed。PathObject target 比较只用于确认下载对象是上级协议对象指定的 ObjId。

`index.json` 最大 256 KiB，manifest 最大 1 MiB，单个 provider metadata
最大 64 MiB；manifest 引用的全部 provider metadata 实际大小之和最大
512 MiB。AICC 在 NDN SDK 完成下载和校验后，以落盘文件的实际大小执行这些容量限制。

## 7. 增量计划与原子提交

客户端把最新有效 activation 与 manifest 比较：

- revision 和 ObjId 相同且本地对象可读：复用，不下载。
- 新 provider、revision 提高或本地对象丢失：整文件下载。
- tombstone 提高：新 activation 不再引用该 provider，不下载文件。
- revision 回退、同 revision 不同 ObjId、无 tombstone 删除：拒绝整个候选。

严格验证过的 index/manifest 会先推进 observed 水位；即使随后某个 provider body 校验失败，已观察到的 manifest revision、provider revision 和 tombstone revision 也不能回退。发布端可以保留相同 ObjId 修复传输，或发布更高 revision 的修复版本。

所有文件准备完成后写一个新的不可变 activation。activation 是唯一提交标记，引用内容寻址的只读对象，并保存 manifest 的 SHA-256 用于检测本地 wrapper 被部分改写；这不是对 NDN 下载内容的二次校验。activation 通过同目录临时文件 `sync_all` 后，以原子的 create-if-absent 操作提交到一个此前不存在的 revision 文件名，不覆盖旧 activation；Unix 使用 hard link 并同步父目录，Windows 使用 `MOVEFILE_WRITE_THROUGH`。

例如只有 `openai.json` 变化时，请求是 `index.json + manifest.json + openai.json`；其他 provider 对象直接复用。

## 8. 中断、恢复与退避

启动和每次尝试前都删除 staging 和 `.part`。最新 observed manifest 是正在更新的
candidate：已经完整下载并通过 NDN 与 JSON 身份校验的 provider 对象可以跨重试复用，
但重试仍从 index 和 manifest 开始，不恢复下载中间进度。既不被保留 activation、也不被
最新 candidate 引用的对象属于垃圾并立即清理，不存在持久化的 `updating` 锁或阶段机。
settings 发生变化时立即取消当前下载并删除本次 staging，然后按新 settings 重新调度；旧 source 的慢响应不能阻塞切源或禁用。

读取端首次按 revision 从高到低验证 activation 及其全部对象，并在进程内缓存已完整验证的最高版本。普通读取复核 activation wrapper 和当前 provider 对象；目标对象损坏时清除缓存，重新执行完整验证并回退到前一份，均不可用时回退内置 metadata。旧 activation 不会被候选就地修改，因此任何断电点都不会产生半生效配置。

activation 提交后立即对所有已注册 Provider 执行一次 best-effort inventory refresh，并把
成功结果直接写入 ModelRegistry；单个 Provider 刷新失败只记录错误，不阻断其它 Provider，
后续仍可通过周期刷新、`provider.refresh_models`、settings reload 或服务重启恢复。activation
提交、同 revision 缓存修复、LKGS 降级、全部 activation 失效或更新源切换都会在实际生效
identity 变化时推进进程内的 `driver_metadata_generation`；identity 包含 source、manifest
revision、ObjId 和 digest。重新解析 metadata 后生成的 inventory 携带该 generation。
ModelRegistry 在 generation 提高时必须替换 provider 快照，
即使 provider 返回的 `inventory_revision` 没有变化；旧 generation 的迟到库存不得覆盖新快照。
所有内置 Provider 都在注册后启动相同生命周期的库存刷新任务；没有远端模型列表接口的
Provider 只按现有 settings 模型列表重新应用 metadata，不额外访问网络。
Claude 和 Gemini 模型列表分页最多读取 10 页，cursor/page token 不能为空且不能重复，并通过 URL query 编码传递；超过边界时本次刷新失败并保留当前 inventory。OpenAI 模型发现当前是单次 `/models` 请求，不存在客户端分页循环。

连续失败采用带 jitter 的指数退避，默认从 60 秒开始，最大不超过配置的正常更新周期；成功后清零。退避只影响调度，不改变安全校验。

## 9. 兼容规则

- v1 冻结字段：format、major version、revision 单调语义、provider identity、ObjId 绑定和 tombstone 语义。
- 可扩展字段：只有具备明确缺省行为的可选字段可以通过 revision 增加。
- 未知 major、未知 required feature、未知 metadata schema major 均 fail-closed，并继续使用 LKGS。
- 至少在一个发布支持窗口内继续维护旧 major 的 manifest；新客户端可另行选择新 major，旧客户端保持旧轨道。
