# AICC Driver Metadata Update 持久化数据格式

## 1. Overview

服务：AICC。协议见 [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)。

AICC 持久保存已验证的发布水位、provider metadata 对象和已提交 activation，用于防回滚、断电恢复和 LKGS 回退。

## 2. Data Classification

| 数据项 | 分类 | 生命周期 |
|---|---|---|
| observed index/manifest 水位 | Durable | 跨重启、安装覆盖和升级保留 |
| provider metadata objects | Durable | 被保留 activation 或最新 candidate 引用时保留 |
| activation | Durable | 当前 LKGS 及一个并发读取/回退版本保留 |
| staging、`.part` | Disposable | 启动及失败时整体删除 |
| 未引用对象 | Disposable | mark-and-sweep 删除 |
| 退避计数 | Disposable | 进程重启后可重新开始 |

## 3. Storage Strategy

位置：`$BUCKYOS_ROOT/data/srv/aicc/driver_metadata/remote_cache/v1/<source-key>/`。
`source-key` 是 canonical `source_url` 的 SHA-256，不同发布源的防回滚水位和对象严格隔离。

```text
<source-key>/objects/<FileObject-ObjId>.json
<source-key>/objects/<FileObject-ObjId>.sha256
<source-key>/activations/<manifest-revision>.json
<source-key>/observed/index/<index-revision>.json
<source-key>/observed/manifest/<manifest-revision>.json
<source-key>/staging/<attempt>/...
```

这是文件系统直接作为核心数据模型的显式例外，风险为目录损坏和跨文件提交。选择该方式的原因是数据都是小型、不可变、内容寻址的 NDN 文件，没有结构化查询；activation 单文件是唯一提交点，避免 RDB head 与运行时文件之间的双提交。所有 durable 文件都先写同目录临时文件、`sync_all`，再用原子的 create-if-absent 操作提交到此前不存在的最终路径：Unix 使用 hard link 并同步父目录，Windows 使用 `MOVEFILE_WRITE_THROUGH`。文件系统布局不进入用户配置或 API，未来可迁移到对象存储。

## 4. Schema Definitions

### Object: observed index

- 文件名：`<index_revision_seq>.json`
- 内容：经 NDN SDK 下载并验证、且通过协议解析的完整 index，加 `path_obj_id`。
- 约束：revision 单调；同名文件内容冲突时 fail-closed。

### Object: observed manifest

- 文件名：`<revision_seq>.json`
- 内容：经 NDN SDK 下载并验证、且通过协议解析的完整 manifest，加 index 声明的 manifest ObjId。
- 约束：revision 单调；同 revision 不同 ObjId fail-closed。

### Object: provider metadata

- 文件名：FileObject ObjId 加 `.json`。
- 内容：协议定义的 UTF-8 JSON。
- 约束：ObjId 已由 NDN SDK 校验；首次落盘同时保存内容 SHA-256，缓存复用时重新计算并匹配，用于发现落盘后的静默损坏；内部 provider、schema、revision 与 manifest 一致。
- 大小：单对象不超过 64 MiB；单 manifest 的对象总大小不超过 512 MiB。

### Object: activation

- 文件名：`<manifest revision_seq>.json`。
- 内容：已接受 manifest、manifest ObjId 及 manifest SHA-256。
- 约束：文件名必须等于 manifest `revision_seq`；首次选择该 activation 时验证 wrapper、manifest SHA-256、协议字段及全部 provider object，命中进程内验证缓存后只复核 wrapper 和当前 provider object。只有引用对象全部落盘且可解析后才能创建；文件创建即提交。

## 5. Schema Version

目录和本地 wrapper 的初始版本为 `v1`。provider metadata 的 `schema_version`、分发协议的 `protocol_version` 与本地目录版本相互独立。

## 6. Upgrade Compatibility Strategy

| 数据项 | 策略 |
|---|---|
| v1 observed 水位 | Additive-only；冻结 revision/ObjId 语义 |
| v1 provider object | Rebuild；可按相同 ObjId 从发布端重新获取 |
| v1 activation | Rebuild；缺少必需 manifest 摘要的旧 activation 视为无效并重新下载 |
| staging | Rebuild；任何升级都可删除 |

不兼容本地布局使用新目录版本并从旧 activation 一次性导入；导入失败继续读旧目录，不原地改写旧状态。

## 7. Extensibility Rules

冻结：revision 比较、ObjId、provider_driver、activation 提交语义。可扩展：wrapper 中具有缺省行为的诊断字段。核心对象不提供任意 `extra`，避免未知字段被误认为安全语义。

## 8. Query Patterns

| 查询 | 支持方式 |
|---|---|
| 最新 observed revision | 枚举小型目录并取最大 revision |
| 最新可用 activation | revision 降序验证文件及引用对象；进程内缓存已完整验证的最高版本，普通读取只复核 activation wrapper 和目标 provider 对象，目标损坏时清除缓存并重新执行完整 LKGS 选择 |
| 按 ObjId 读取 provider metadata | 文件名直接定位 |
| 清理孤儿 | activation 引用集合与 objects 目录做差 |

单个 source namespace 的目录规模由保留策略限制，不存在大规模扫描或复杂索引。

每个 source namespace 最多保留两个有效 activation、两个 index 水位和两个 manifest
水位；对象保留集合是这些 activation 与最新 observed manifest 的引用并集。它覆盖当前
生效版本、一次必要回退/并发读取版本和正在更新的 candidate。其它对象、旧水位、旧
activation 和 staging 均清理。未使用 source namespace 保留其 LKGS、对象和 observed 水位，防止停用或切源后丢失防回滚状态；因此单个 source 的占用有确定上界，总占用还取决于历史配置过的 source 数量。删除历史 source namespace 只能由显式维护操作完成。
