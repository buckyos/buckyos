# AICC Driver Metadata Update 持久化数据格式

## 1. Overview

服务：AICC。协议见 [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)。

AICC 持久保存已验证的发布水位、provider metadata 对象和已提交 activation，用于防回滚、断电恢复和 LKGS 回退。

## 2. Data Classification

| 数据项 | 分类 | 生命周期 |
|---|---|---|
| observed index/manifest 水位 | Durable | 跨重启、安装覆盖和升级保留 |
| provider metadata objects | Durable | 被保留 activation 引用时保留 |
| activation | Durable | 当前 LKGS 及回退版本保留 |
| staging、`.part` | Disposable | 启动及失败时整体删除 |
| 未引用对象 | Disposable | mark-and-sweep 删除 |
| 退避计数 | Disposable | 进程重启后可重新开始 |

## 3. Storage Strategy

位置：`$BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/v1/`。

```text
objects/<FileObject-ObjId>.json
activations/<manifest-revision>.json
observed/index/<index-revision>.json
observed/manifest/<manifest-revision>.json
staging/<attempt>/...
```

这是文件系统直接作为核心数据模型的显式例外，风险为目录损坏和跨文件提交。选择该方式的原因是数据都是小型、不可变、内容寻址的 NDN 文件，没有结构化查询；activation 单文件是唯一提交点，避免 RDB head 与运行时文件之间的双提交。所有 durable 文件都先写同目录临时文件、`sync_all`，再用原子的 create-if-absent hard link 提交到此前不存在的最终路径。

## 4. Schema Definitions

### Object: observed index

- 文件名：`<index_revision_seq>.json`
- 内容：完整且已严格验证的 index，加 `path_obj_id`。
- 约束：revision 单调；同名文件内容冲突时 fail-closed。

### Object: observed manifest

- 文件名：`<revision_seq>.json`
- 内容：完整且已严格验证的 manifest，加 index 声明的 manifest ObjId。
- 约束：revision 单调；同 revision 不同 ObjId fail-closed。

### Object: provider metadata

- 文件名：FileObject ObjId 加 `.json`。
- 内容：协议定义的 UTF-8 JSON。
- 约束：ObjId 已由 NDN SDK 校验；内部 provider、schema、revision 与 manifest 一致。

### Object: activation

- 文件名：`<manifest revision_seq>.json`。
- 内容：已接受 manifest 的原文及 manifest ObjId。
- 约束：只有它引用的全部 provider object 已落盘且可解析后才能创建；文件创建即提交。

## 5. Schema Version

目录和本地 wrapper 的初始版本为 `v1`。provider metadata 的 `schema_version`、分发协议的 `protocol_version` 与本地目录版本相互独立。

## 6. Upgrade Compatibility Strategy

| 数据项 | 策略 |
|---|---|
| v1 observed 水位 | Additive-only；冻结 revision/ObjId 语义 |
| v1 provider object | Rebuild；可按相同 ObjId 从发布端重新获取 |
| v1 activation | Additive-only；旧 reader 必须能忽略新增可选字段 |
| staging | Rebuild；任何升级都可删除 |

不兼容本地布局使用新目录版本并从旧 activation 一次性导入；导入失败继续读旧目录，不原地改写旧状态。

## 7. Extensibility Rules

冻结：revision 比较、ObjId、provider_driver、activation 提交语义。可扩展：wrapper 中具有缺省行为的诊断字段。核心对象不提供任意 `extra`，避免未知字段被误认为安全语义。

## 8. Query Patterns

| 查询 | 支持方式 |
|---|---|
| 最新 observed revision | 枚举小型目录并取最大 revision |
| 最新可用 activation | revision 降序验证文件及引用对象 |
| 按 ObjId 读取 provider metadata | 文件名直接定位 |
| 清理孤儿 | activation 引用集合与 objects 目录做差 |

目录规模由保留策略限制，不存在大规模扫描或复杂索引。
