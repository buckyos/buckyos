# AICC Provider Metadata 云更新协议

版本：`v2`
状态：Beta 2.2 目标规范

本文定义 Model Driver、Provider Rules 和 Known Provider 三类 catalog 的发布结构、客户端版本选择、NDN 文件更新与 AICC 库存收敛边界。Metadata schema 见 [driver_metadata_schema.md](driver_metadata_schema.md) 和 [provider_profile_schema.md](provider_profile_schema.md)，持久化边界见 [driver_metadata_update_storage.md](driver_metadata_update_storage.md)。

## 1. 边界

- Model Driver catalog 保存模型固有能力、家族、variant 和逻辑挂载。
- Provider Rules catalog 保存渠道模型映射、operation、请求规则、能力收窄及价格规则；价格不再单独发布 Pricing Catalog。
- Known Provider catalog 保存管理 UI 使用的服务商默认 endpoint、Profile 和 Adapter。
- Provider Instance 名称、endpoint、凭据、区域、账号和协议选择属于 system-config，catalog 无权修改。
- Provider discovery 产生的 availability、deprecated、remote methods、实时价格和 health 属于实例级动态事实，不写入静态 catalog。
- 三类 catalog 使用独立文件和 revision，但通过同一个 manifest 发布为完整版本。
- 发布结构与文件交付由 NDN 更新链路负责；AICC 不重复实现文件下载、验签、完整性校验或 activation。

## 2. 发布路径

```text
/aicc/provider-catalog/index.json
/aicc/provider-catalog/v2/manifest-<revision_seq>.json
/aicc/provider-catalog/v2/model-drivers/<id>-<revision_seq>.json
/aicc/provider-catalog/v2/provider-rules/<id>-<revision_seq>.json
/aicc/provider-catalog/v2/known-providers/<id>-<revision_seq>.json
```

发布顺序固定为：catalog 内容文件、manifest、index。Index 最后更新，客户端不得扫描目录猜测最新版本。这里没有 `pricing/` 路径；渠道静态价格及条件计价规则直接包含在 Provider Rules 的 `models[].pricing` / `patterns[].pricing` 中。

## 3. Index

Index 格式为 `buckyos.aicc.provider-catalog-index`，至少包含：

- `index_version: 2`；
- 严格递增的 `index_revision_seq`；
- `tracks[]`：可投放版本列表。

每个 track 至少声明：

- manifest 的 `revision_seq`；
- manifest 的 path 和对象身份；
- `match: MatchRule`，字符串简写匹配客户端版本；需要联合版本、更新通道或灰度分组时才使用对象；
- `required_features`；
- 可选更新通道和灰度分组条件包含在 `match` 对象中。

`MatchRule` 统一遵循 [match_rule.md](match_rule.md)。单一客户端版本范围保持简写，例如 `"2.2.*"`；多维投放才写成 `{ "client_version": "2.2.*", "update_channel": "stable", "rollout_group": "cn-*" }`，不为普通发布强制填写多层条件对象。

云更新服务可以给不同客户端版本、更新通道或灰度分组配置不同 track。NDN 更新链路只能选择与本机客户端兼容的目标，不兼容或包含未知 required feature 的 track 必须拒绝。

## 4. Manifest

Manifest 格式为 `buckyos.aicc.provider-catalog-manifest`，描述一个完整可用的发布版本，至少包含：

- `protocol_version: 2`；
- 全局唯一、严格递增且不可复用的 `revision_seq`；
- 与 index track 一致的客户端兼容范围和 `required_features`；
- `files[]`：`catalog_kind`、`catalog_id`、path、schema version、对象 revision 和对象身份；
- `tombstones[]`：从完整发布集合中删除的 catalog kind/id 及其 revision。

`catalog_kind` 只允许 `model_driver`、`provider_rules`、`known_provider`。`catalog_kind + catalog_id` 在 manifest 内唯一；未变化文件可以保持自己的 revision 和对象身份，删除必须使用 tombstone。Manifest 指向的三类文件合起来构成该 `revision_seq` 的完整 metadata 文件集合。

## 5. 发布文件内容

### 5.1 Model Driver

路径：`v2/model-drivers/<model_driver_id>-<revision_seq>.json`。

内容定义原厂模型的静态技术语义，包括 API type、capability、家族、版本规则、variant、逻辑挂载和保守成本估值。不得包含 Provider endpoint、认证、渠道 operation、请求参数或实例动态状态。

### 5.2 Provider Rules

路径：`v2/provider-rules/<provider_profile_id>-<revision_seq>.json`。

内容定义渠道模型到原厂 Model Driver/ModelUID 的映射，以及 operation、provider options、request rules、能力收窄、价格和条件计价规则。价格使用规则内的 `pricing` 字段，不使用独立 `pricing_ref` 或 Pricing Catalog。

### 5.3 Known Provider

路径：`v2/known-providers/<catalog_id>-<revision_seq>.json`。

内容定义管理 UI 使用的已知服务商默认值，包括 `provider_profile_id`、显示名称、默认 endpoint、`protocol_adapter_id`、可选 `provider_rules_id` 和 UI hints。它不能修改已经存在的 Provider Instance 私有配置。

## 6. 版本兼容与防回退

- 每个完整发布版本使用 manifest 中全局唯一且严格递增的 `revision_seq`；允许跳号，不允许复用序列或覆盖同序列内容。
- 本机已接受的 `metadata_target_seq` 是持久高水位。NDN 只能接受更大的兼容 manifest `revision_seq`；更小序列必须拒绝，相同序列但内容不同视为发布冲突。
- 云端可以同时保留多个 track：旧客户端取得其兼容轨道上的最新版本，新客户端可以取得使用新 schema/feature 的更高版本。
- 云端修改某组客户端的目标时，新目标仍必须高于该客户端本机水位。恢复旧内容必须把旧内容重新发布为更高序列的新版本，不能通过普通更新降低序列。

兼容性选择、发布文件校验和防回退由 NDN 更新链路保证。AICC 不重复验证 NDN 已交付文件的签名、ObjId、digest、manifest、兼容范围或版本水位；保证不足时应向 NDN 提交 bug。

## 7. 更新时序

```text
读取 index，并按客户端版本/通道/灰度分组选择兼容且 revision_seq 更高的 manifest
  -> 下载并校验 manifest 指定的完整 catalog 文件集合
  -> 替换当前 metadata 文件并确认新文件已可供 Provider 应用
  -> 发布 metadata_target_seq = manifest.revision_seq
  -> 收到下一次 AICC 推理请求，或进入任一 Provider Instance 定时库存刷新
  -> 统一加载 target_seq 对应的全部 metadata 更新
  -> 收敛所有 applied_seq != target_seq 的 Provider inventory
  -> 每个 Provider 真正完成库存刷新后提交 applied_seq = 本次捕获的 target_seq
  -> 继续原推理或定时库存刷新
```

在 manifest 和全部 catalog 文件下载、校验、替换完成并已可供 Provider 应用之前，NDN 不得推进 `metadata_target_seq`。任一步失败时保持原文件和原目标序列；只有新文件就绪后才提交 `metadata_target_seq = manifest.revision_seq`。

`metadata_target_seq` 持续存在且只递增，不在应用完成后清除。是否完成应用由各 Provider 的 `metadata_applied_seq` 与当前目标是否相等判断。

## 8. NDN 更新链路责任

- 读取并验证 index、所选 track、manifest 和 manifest 指定的完整文件集合；
- 根据客户端版本、更新通道和灰度分组选择兼容 track；
- 检查 manifest `revision_seq` 高于本机已接受水位；没有更高兼容版本时保持现状；
- 保证文件来源可信、内容完整、版本匹配、引用一致且集合可用；
- 替换当前文件集合并确认新文件可供 Provider 加载；
- 仅在文件就绪后发布 `metadata_target_seq = manifest.revision_seq`。

版本不兼容、序列未前进，或下载、校验、替换、就绪确认任一步失败时，都不得推进目标序列。签名、ObjId、digest、断点续传、具体替换方法和失败恢复属于 NDN 实现；本协议只固定发布结构及交付结果。

## 9. AICC 触发时机

以下两个入口在执行自身逻辑前读取 `metadata_target_seq`：

- 推理请求进入路由和 Provider 选择前；
- 任一具体 Provider Instance 开始定时库存刷新时。

任一入口发现至少一个 Provider 的 `metadata_applied_seq` 与目标不一致时，都启动同一个全局收敛过程。定时任务只提供触发机会，不能把更新范围缩小为当前 Provider。

Provider Instance 停止、禁用、删除、被 reload 替换或随 AICC 服务退出时，必须向其库存刷新定时任务循环发送 `Stop` 事件并等待优雅退出。循环收到停止事件后不得再发起新的定时探测或 metadata 收敛，也不得在实例停止后提交迟到结果；这只关闭该实例的定时触发源，不改变其它入口触发时“统一处理全部序列落后 Provider 库存”的范围。

## 10. Provider 库存收敛

全局收敛先加载本次捕获的目标序列对应的完整 metadata snapshot，再遍历所有 Provider：

1. 读取 Provider 当前 `metadata_applied_seq` 和已保存的 provider model 列表。
2. 获取本轮可用的 provider model 列表：定时刷新触发的 Provider 使用刚探测的列表，其它 Provider 可以使用已保存列表。
3. 若 `metadata_applied_seq != metadata_target_seq`，把本次捕获的目标临时记录为 `metadata_updating_seq`。
4. 使用同一完整 metadata snapshot 重建该 Provider inventory。
5. 真正完成库存刷新后，原子提交新 inventory 和 `metadata_applied_seq = metadata_updating_seq`，再清除临时值。
6. 未完成或失败时保持该 Provider 原 inventory 和 `metadata_applied_seq`，不得把目标序列记为已应用；后续触发仍会发现不一致并重试。

`metadata_updating_seq` 不是已应用状态。若刷新期间 NDN 又推进目标序列，本轮只提交自己开始时捕获的序列；完成后它仍与新目标不一致，下一轮继续收敛，不能把新目标误记为已应用。

新建 Provider 没有旧库存时，首次 discovery 必须捕获当前目标序列，并把首份 inventory 与对应 `metadata_applied_seq` 一起提交。

## 11. 无事实更新的探测

Provider 定时库存刷新时，同时比较 provider model 列表和 metadata 序列：

| model 列表 | `metadata_applied_seq` 与目标 | 行为 |
| --- | --- | --- |
| 未变化 | 相同 | 仅完成连通性/健康探测，不重建或重写 inventory |
| 已变化 | 相同 | 按新 model 列表更新该 Provider inventory，序列保持不变 |
| 未变化 | 不同 | 必须按目标 metadata 重建 inventory，并在成功后推进 `metadata_applied_seq` |
| 已变化 | 不同 | 使用新 model 列表和目标 metadata 重建 inventory，并在成功后推进 `metadata_applied_seq` |

因此，列表未变化不能替代 metadata 序列比较；只有“列表未变化且序列相同”才是真正的 no-op probe。

## 12. 全局一致性与并发

- 一次收敛使用同一个已捕获 `target_seq` 和完整 metadata snapshot。
- 必须遍历所有序列落后的 Provider，不能先预测本次调用会命中哪个 Provider。
- 并发触发合并为一个刷新执行者；其它请求等待结果或复用已完成结果。
- 推理进入路由前，所有参与路由的 Provider 必须已收敛到本次捕获的目标序列。
- 某个 Provider 失败时保持其原 inventory 和旧 `metadata_applied_seq`，记录可诊断错误，不得把目标序列或部分刷新结果标记为成功。

## 13. 非目标

AICC 不负责 NDN 文件二次验签、下载缓存、candidate/activation、按请求加载部分 metadata，或为 metadata 另建后台定时更新任务。AICC 也不维护独立 Pricing Catalog；静态价格规则属于 Provider Rules。云端 track 配置、客户端兼容匹配和防回退高水位属于 NDN 更新链路。
