# AICC Metadata 更新持久化边界

## 1. Overview

服务：AICC。流程见 [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)。

Metadata 文件和全局目标序列由 NDN 管理。AICC 不维护对象缓存、activation、更新专用 LKGS、revision 水位、manifest 副本或 staging 目录；AICC 只保存每个 Provider inventory 已应用的序列。

## 2. Data Classification

| 数据项 | 所有者 | 生命周期 |
| --- | --- | --- |
| 当前 metadata 文件集合 | NDN | 由 NDN 下载、校验和替换 |
| `metadata_target_seq` | NDN | Durable，指向当前文件版本的目标序列 |
| `metadata_applied_seq` | AICC Provider inventory | Durable，该 Provider 已正式应用的目标序列 |
| `metadata_updating_seq` | AICC 刷新过程 | Transient，单次刷新开始时捕获的目标序列 |
| metadata resolver/catalog snapshot | AICC | Memory，可由当前文件重建 |
| provider model 列表及其 inventory | AICC 既有库存机制 | 定时探测或 metadata 序列变化时更新 |
| Provider 库存刷新定时任务、控制通道 | AICC | Memory，实例启动时创建，停止时发送 `Stop` 并等待循环退出 |

不新增 AICC 专用 remote cache、candidate、activation、observed revision、回滚版本或对象 digest 文件。

## 3. 序列字段语义

### `metadata_target_seq`

类型为非负 `u64`。NDN 仅在完整 metadata 文件替换成功后推进。AICC 只做相等性比较，不自行推导、递增或校验文件版本。

### `metadata_applied_seq`

类型为非负 `u64`，每个 Provider inventory 独立保存。它只能与使用对应 metadata snapshot 构造的新 inventory 一起成功提交，不能在刷新开始时提前修改。

### `metadata_updating_seq`

刷新开始时从目标序列复制得到，只表示本轮准备应用哪个版本，不表示已经成功。它优先作为进程内临时状态；进程崩溃后不需要恢复，因为持久的 `metadata_applied_seq` 仍保持旧值，下一次触发会重新收敛。

## 4. Provider Inventory Schema

Provider inventory 至少记录：

```text
provider_instance_name
provider_model_list
provider_model_list_fingerprint
metadata_applied_seq
inventory payload
updated_at
```

`provider_model_list_fingerprint` 只用于判断本次 discovery 列表是否发生变化，不承担 metadata 文件完整性校验。

提交规则：

1. 开始重建前设置临时 `metadata_updating_seq = metadata_target_seq`。
2. 使用该序列对应的完整 metadata snapshot 构造 inventory。
3. 成功时原子写入 inventory、model list/fingerprint 和 `metadata_applied_seq = metadata_updating_seq`。
4. 失败时保持原 inventory 和 `metadata_applied_seq`，丢弃临时值。

新建 Provider 的首份 inventory 直接捕获并应用当前 `metadata_target_seq`，不得以空值表示已经同步。

## 5. 触发与 no-op

推理请求前或任一 Provider Instance 定时库存刷新时，发现任何 Provider 的 `metadata_applied_seq != metadata_target_seq`，都执行全局收敛。

定时探测某个 Provider 时：

- model 列表未变化且序列相同：仅探测，不重写 inventory；
- model 列表变化或序列不同：必须更新该 Provider inventory；
- 序列不同的其它 Provider 也必须在同一全局过程内收敛，不能只更新当前 Provider。

定时任务和控制通道不持久化。Provider 停止、禁用、删除、被配置 reload 替换或服务退出时，先禁止新刷新，再通过控制通道发送幂等 `Stop` 事件并等待任务循环优雅退出；退出后的任务不得提交 inventory、fingerprint、applied seq 或 health。Provider 再次启用后创建新循环，并根据持久 inventory 的 model fingerprint 和 `metadata_applied_seq` 继续收敛。

## 6. 并发与故障

- 一轮刷新捕获一个目标序列；目标在刷新中再次变化时，不修改本轮捕获值。
- Provider 成功提交旧捕获值后若仍落后于最新目标，下一轮继续刷新。
- AICC 无法加载 NDN 文件或重建某个 Provider inventory 时，不推进该 Provider 的 `metadata_applied_seq`。
- 如果故障来自 NDN 未能保证文件契约，应向 NDN 提交 bug，而不是在 AICC 中增加校验、activation、LKGS 或回滚流程。

## 7. Beta 2.2 兼容策略

Beta 2.2 是 breaking change。旧布尔 `metadata_update_pending`、`remote_cache/v1`、`provider_catalog/remote_cache/v2`、activation、observed water mark 和 staging 数据不读取、不导入，也不提供兼容迁移。
