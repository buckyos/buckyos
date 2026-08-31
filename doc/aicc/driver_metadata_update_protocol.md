# AICC Metadata 云更新协议

版本：`v2`
状态：Beta 2.2 目标规范

本文只定义 NDN 文件更新与 AICC 库存收敛之间的最小边界。Metadata schema 见 [driver_metadata_schema.md](driver_metadata_schema.md)，持久化边界见 [driver_metadata_update_storage.md](driver_metadata_update_storage.md)。

## 1. 设计约束

1. Metadata 文件的版本发现、下载、完整性、真实性、版本一致性和本地替换全部由 NDN 保证。
2. AICC 不重复验证 NDN 已交付文件的签名、ObjId、digest、manifest、版本水位或下载完整性；保证不足时应向 NDN 提交 bug。
3. NDN 替换文件后发布全局目标序列号 `metadata_target_seq`，不设置一次性布尔更新标记。
4. 每个 Provider inventory 持久记录 `metadata_applied_seq`，表示该库存实际应用的 metadata 目标序列。
5. 推理请求前或任一 Provider Instance 定时库存刷新时可以触发收敛；无论由谁触发，都处理所有 `metadata_applied_seq != metadata_target_seq` 的 Provider，不按请求依赖或当前 Provider 局部更新。
6. AICC 不设计 metadata activation、staging、更新专用 LKGS、回滚水位或按发布源隔离的对象缓存。

## 2. 唯一更新流程

```text
发现更新版本
  -> NDN 下载并保证更新版本正确
  -> NDN 替换旧 metadata 文件
  -> NDN 发布 metadata_target_seq
  -> 收到下一次 AICC 推理请求，或进入任一 Provider Instance 定时库存刷新
  -> 统一加载 target_seq 对应的全部 metadata 更新
  -> 收敛所有 applied_seq != target_seq 的 Provider inventory
  -> 每个 Provider 成功后提交 applied_seq = 本次捕获的 target_seq
  -> 继续原推理或定时库存刷新
```

`metadata_target_seq` 是持续存在的目标水位，不在应用完成后清除。是否已完成由各 Provider 的 `metadata_applied_seq` 与当前目标是否相等判断。

## 3. NDN 责任

NDN 更新端负责：

- 发现远端最新版本；
- 下载该版本包含的完整 metadata 文件集合；
- 保证来源可信、内容完整、版本匹配且文件集合可用；
- 用新版本替换本地旧版本；
- 仅在替换成功后发布与该文件版本对应的 `metadata_target_seq`。

具体 NDN 路径、对象组织、签名、digest、断点续传、原子替换和失败恢复均属于 NDN。下载或替换失败时不得推进目标序列，AICC 继续使用现有序列状态。

## 4. AICC 触发时机

以下两个入口在执行自身逻辑前读取 `metadata_target_seq`：

- 推理请求进入路由和 Provider 选择前；
- 任一具体 Provider Instance 开始定时库存刷新时。

任一入口发现至少一个 Provider 的 `metadata_applied_seq` 与目标不一致时，都启动同一个全局收敛过程。定时任务只提供触发机会，不能把更新范围缩小为当前 Provider。

Provider Instance 停止、禁用、删除、被 reload 替换或随 AICC 服务退出时，必须向其库存刷新定时任务循环发送 `Stop` 事件并等待优雅退出。循环收到停止事件后不得再发起新的定时探测或 metadata 收敛，也不得在实例停止后提交迟到结果；这只关闭该实例的定时触发源，不改变其它入口触发时“统一处理全部序列落后 Provider 库存”的范围。

## 5. Provider 库存收敛

全局收敛先加载本次捕获的目标序列对应的完整 metadata snapshot，再遍历所有 Provider：

1. 读取 Provider 当前 `metadata_applied_seq` 和已保存的 provider model 列表。
2. 获取本轮可用的 provider model 列表：定时刷新触发的 Provider 使用刚探测的列表，其它 Provider 可以使用已保存列表。
3. 若 `metadata_applied_seq != metadata_target_seq`，把本次捕获的目标临时记录为 `metadata_updating_seq`。
4. 使用同一完整 metadata snapshot 重建该 Provider inventory。
5. 成功后原子提交新 inventory 和 `metadata_applied_seq = metadata_updating_seq`，再清除临时值。
6. 失败时不得推进 `metadata_applied_seq`；后续触发仍会发现不一致并重试。

`metadata_updating_seq` 不是已应用状态。若刷新期间 NDN 又推进目标序列，本轮只提交自己开始时捕获的序列；完成后它仍与新目标不一致，下一轮继续收敛，不能把新目标误记为已应用。

新建 Provider 没有旧库存时，首次 discovery 必须捕获当前目标序列，并把首份 inventory 与对应 `metadata_applied_seq` 一起提交。

## 6. 无事实更新的探测

Provider 定时库存刷新时，同时比较 provider model 列表和 metadata 序列：

| model 列表 | `metadata_applied_seq` 与目标 | 行为 |
| --- | --- | --- |
| 未变化 | 相同 | 仅完成连通性/健康探测，不重建或重写 inventory |
| 已变化 | 相同 | 按新 model 列表更新该 Provider inventory，序列保持不变 |
| 未变化 | 不同 | 必须按目标 metadata 重建 inventory，并在成功后推进 `metadata_applied_seq` |
| 已变化 | 不同 | 使用新 model 列表和目标 metadata 重建 inventory，并在成功后推进 `metadata_applied_seq` |

因此，列表未变化不能替代 metadata 序列比较；只有“列表未变化且序列相同”才是真正的 no-op probe。

## 7. 全局一致性与并发

- 一次收敛使用同一个已捕获 `target_seq` 和完整 metadata snapshot。
- 必须遍历所有序列落后的 Provider，不能先预测本次调用会命中哪个 Provider。
- 并发触发合并为一个刷新执行者；其它请求等待结果或复用已完成结果。
- 推理进入路由前，所有参与路由的 Provider 必须已收敛到本次捕获的目标序列。
- 某个 Provider 失败时保留其旧 `metadata_applied_seq` 并返回可诊断错误，不得把部分重建结果标记为成功。

## 8. 非目标

AICC 不负责 NDN 文件二次验签、index/manifest/ObjId 协议、candidate/activation、水位回滚、多版本缓存、按请求加载部分 metadata，或为 metadata 另建后台定时更新任务。
