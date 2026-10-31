# 本地修复后定向复测结果

本轮仅重测原本受本地 SQLite/异步驱动故障影响的三个单元，不重跑或重判此前 21 项识别质量失败。GitHub 已提交并回读核验：[补充 #49](https://github.com/buckyos/sn-business/issues/49#issuecomment-6527012935)、[Terra #53](https://github.com/buckyos/sn-business/issues/53)。

## 验证结论

- AICC 公共存储/驱动修复已构建并部署，运行数据库已确认为 WAL；未新增第三方依赖。
- Rust 单测 789/789，acceptance 自测 150/150，相关 Deno 类型检查及 release 构建通过。
- [定向 T1.5](../t15-20261030155506-1454156/summary.md)：30/30 通过，随后才执行 T2。范围包含相关模型 OCR/生图/编辑、异步状态、取消、错误响应与 TaskMgr artifact。
- [定向 T2 原始报告](../aicc-2026-10-30T15-56-30-584Z-312e474a/summary.md)：真实调用 4 次（3 个目标单元及 1 次前置生图），零 runner 自动重试；三个目标任务全部 Terminal/Succeeded。
- T2 原始状态为 review=3；另有 cleanup failed=1。不能将进程退出码 1 解释为三个目标推理失败，也不能将成功调用冒充全部内容已经验收。

| 目标单元 | TaskMgr | 用例耗时 | 原故障验证 | 内容验收 |
|---|---|---:|---|---|
| gpt-5.4-pro / vision.ocr | Terminal/Succeeded | 381.662s | 已完成异步任务，未再悬挂 | 保留 review，不新增 OCR 质量判分 |
| gpt-5.5-pro / vision.ocr | Terminal/Succeeded | 348.038s | 已完成异步任务，未再悬挂 | 保留 review，不新增 OCR 质量判分 |
| gpt-5.5 / image.img2img | Terminal/Succeeded | 40.624s | 前置生成、NDN 读取、归属登记、img2img 及最终制品读取全部成功 | 人工比较通过：保留蓝色方块/4827 构图并应用暖色光照 |

Task：GPT-5.4-Pro OCR `t-865eefb3efcd402d90098ed0206c3d91`；GPT-5.5-Pro OCR `t-b551ea3f94ef49adbbe4f6d27ce9b56e`；GPT-5.5 img2img `t-71c5ba5e9262496182423ad8fc4c26bd`。前置生图为 `t-245528927ad242cd930f4f9e80c2cacc`。

## 归属登记与制品

前置图与最终图均存在 aicc_artifact_scope 记录，tenant/user 均为本次授权用户；没有通过跳过归属校验修复。最终 PNG 为 1254×1254、1,892,542 字节，SHA256 `c26511767dfed7abe1ab1f0583bee16bb23a65edafb873cbba03e9605883018d`。源图与结果均从原生 NDN Gateway 路由实际下载并通过文件校验。

runner 新增显式测试配置 `[fixtures] named_data_read_mode="ndn"`，复用现有 cyfs-dir 服务和测试期 Gateway 路由。默认模式仍为 NDM Proxy；没有按模型硬编码，也没有修改 NDN 或 Control Panel 服务逻辑。

## 收尾与成本

临时 Provider 凭据已恢复，NDN 临时路由已移除。NDM Proxy 删除接口仍返回 405，两个本轮测试对象无法自动删除，cleanup 用例如实失败；未把它改成跳过或通过。

runner 预算字段为 US$1，按默认单次调用估算计算的未知成本暴露为 US$0.08，未触发其预算检查。4 次调用均缺少可确认币种的实际成本，这不是实际账单，也不能据此断言实际结算不超过 US$1；详见原轮次 finance.md。

## 范围边界

本轮解决并验证原本地挂起/归属写入路径，不能证明全部 SN provider 已通过，也没有重测 Qwen Image/Wan/Terra。此前 21 项识别质量结果保持不变。当前跨轮矩阵为 100 passed、64 failed、2 review；两项 review 为本轮 Pro OCR 内容，GPT-5.5 img2img 经人工比较计为 passed。历史轮次未改写。

公共组件修改必要性：Provider 参数无法控制本地 SQLite 读写锁，也不能清除持久化异常后遗留的 active 标记。未来 Git 提交/PR 必须明确标识公共 storage/execution 及日志修改；本轮未创建 Git commit/PR。未执行完整 workspace cargo test 或完整 buckyos-build.py。
