# AICC Provider Catalog 更新协议

版本：`v2`
状态：Beta 2.2 目标规范

本文定义 Model Driver、Provider Rules、Pricing 和 Known Provider 四类 catalog 的 NDN 发布、验证、原子生效与 LKGS 规则。对象 schema 和持久布局见 [provider_architecture_durable_data_schema.md](provider_architecture_durable_data_schema.md)。

## 1. 边界

- Catalog 只更新静态模型语义、渠道映射/规则、价格和已知服务商默认信息。
- Provider Instance 名称、endpoint、凭据、区域、账号和协议选择属于 system-config，catalog 无权修改。
- Provider discovery 产生的 availability、deprecated、remote methods、实时价格和 health 属于实例级动态事实，不写入静态 catalog。
- 四类 catalog 使用独立对象和 revision，但通过同一个 manifest 原子发布。
- Beta 2.2 不读取 v1 driver metadata cache，不迁移旧 `provider_driver` 对象。

## 2. 发布路径

```text
/aicc/provider-catalog/index.json
/aicc/provider-catalog/v2/manifest-<revision_seq>.json
/aicc/provider-catalog/v2/model-drivers/<id>-<revision_seq>.json
/aicc/provider-catalog/v2/provider-rules/<id>-<revision_seq>.json
/aicc/provider-catalog/v2/pricing/<id>-<revision_seq>.json
/aicc/provider-catalog/v2/known-providers/<id>-<revision_seq>.json
```

发布顺序必须是内容对象、manifest、index。Index 最后更新，客户端不得扫描目录猜测最新版本。

## 3. Index

Index 固定格式为 `buckyos.aicc.provider-catalog-index`，包含 `index_version=2`、`index_revision`、严格递增的 `index_revision_seq`、`required_features` 和可选择的 protocol tracks。每个 track 指定 manifest path、ObjId、revision 和 required features。

客户端选择自己支持的最高 track。未知 required feature 必须拒绝该 track，不能忽略后继续解析。

## 4. Manifest

Manifest 固定格式为 `buckyos.aicc.provider-catalog-manifest`，一次列出完整 active catalog 集合：

- `protocol_version=2`、`protocol_revision`、`revision_seq`；
- `files[]`：`catalog_kind`、`catalog_id`、`path`、`schema_version`、`revision_seq`、`obj_id`；
- `tombstones[]`：删除对象的 kind、id 和严格递增 revision。

`catalog_kind` 只允许 `model_driver`、`provider_rules`、`pricing`、`known_provider`。`catalog_kind + catalog_id` 在 manifest 内唯一。未变化对象保持 revision 和 ObjId；删除必须使用 tombstone。

## 5. 严格验证

客户端按以下顺序验证，任一步失败均拒绝整个候选 activation：

1. 使用 NDN SDK 验证 index PathObject 的签名、host、path、exp 和 ObjId。
2. 检查 index 防回滚水位和 required features。
3. 按 index 指定的 path/ObjId 获取并验证 manifest。
4. 检查 manifest revision、防回滚、冲突、唯一键和 tombstone。
5. 按 manifest 下载全部对象并验证 PathObject、FileObject、ObjId、字节上限和 UTF-8 JSON。
6. 按 catalog kind 使用严格 schema 解析，拒绝未知字段和身份/revision 不一致。
7. 构建跨 catalog 引用，验证 Model Driver、Provider Rules、Pricing、Known Provider 引用完整。
8. 验证 operation 均存在于运行时 Protocol Adapter registry。
9. 验证 Provider Rules 只能收窄 Model Driver 能力。
10. 构建不可变内存 snapshot 后提交 activation。

Catalog JSON 不包含内嵌 `signature`。真实性只来自 NDN PathObject/FileObject 信任链、manifest ObjId 绑定和 revision 水位。

## 6. 原子 activation

所有对象先写入 staging；完整验证后才创建 activation 文件。Activation 文件是唯一提交点，包含 manifest revision、ObjId、各 catalog 对象摘要和创建时间。提交后以一次内存指针替换使新 snapshot 生效。

任何下载、解析、引用或编译失败都必须保留旧 active activation 和 LKGS，不修改 Provider Instance，并记录失败原因、时间和候选 revision。

## 7. 防回滚与冲突

- index、manifest 和每个 catalog 对象分别维护 observed high-water mark。
- revision 小于水位时拒绝。
- 同 revision 不同 ObjId 视为发布冲突并拒绝。
- Tombstone revision 不得下降或消失。
- 回退只能通过发布更高 revision 的新 manifest 完成。

## 8. 与 Provider discovery 的关系

Catalog activation 后，AICC 使用新静态 snapshot 重新解析各 Provider Instance inventory。动态事实只能与静态能力取交集。

Discovery 成功时原子替换该实例 inventory LKGS；失败时使用该实例最近成功快照或经过验证的内置 default inventory。不同 Provider Instance 的 refresh、失败和 LKGS 相互隔离。

## 9. 管理和观测

管理 API 至少暴露 active revision、最近 attempt/success 时间、失败原因、各 catalog kind/id revision，以及 Provider Instance inventory 使用的 catalog/discovery revision。日志不得包含凭据、原始请求、资源内容或 ProviderState。
