# AICC SN Provider 验收报告

## 最终汇总

- SN 服务端发布目录中的 33 个模型 alias 已全部纳入 `sn.provider.json`：29 个配置为可执行模型，4 个因 SN 公共协议不完整而显式排除。
- 29 个可执行 alias：`gpt-6-astra`、`gpt-6-sol`、`gpt-6-luna`、`gpt-5.5-pro`、`gpt-5.5`、`gpt-5.4`、`gpt-5.4-pro`、`gpt-5.4-mini`、`gpt-5.4-nano`、`gpt-5.3-codex`、`gpt-image-2`、`text-embedding-3-large`、`text-embedding-3-small`、`gpt-4o-mini-transcribe`、`gpt-4o-transcribe-diarize`、`gpt-4o-mini-tts`、`gpt-5.6-sol`、`gpt-5.6-terra`、`gpt-5.6-luna`、`kimi-k3`、`glm-5`、`glm-5.3`、`qwen3.6-plus`、`qwen3.7-plus`、`qwen3.8-max-0902`、`qwen3-embedding-8b`、`qwen-image-3-pro`、`ui-tars-1.5-7b`、`voxtral-small-24b-2507`。
- 4 个排除 alias：`gpt-realtime-2.1`（SN 未公开 Realtime 路由，AICC 也没有对应 canonical API type）；`sora-2`、`sora-2-pro`、`wan-3.0`（SN 只公开视频任务创建接口，没有任务状态查询和结果获取闭环）。
- 原厂模型参数保留在相应 `*.model.json`，SN 的路由、模型 alias、参数收窄及协议差异配置在 `sn.provider.json`。本轮新增了 OpenAI 转写模型 `gpt-4o-transcribe-diarize` 的原厂元数据，并同步覆盖 SN/OpenAI 两个 Provider 的 T1.5。
- T1 本轮全矩阵 189 项：首次 188 项通过、1 项失败；修正后该失败项定向复验通过，合并结果为 189/189，真实模型调用 0。
- T1.5 最终结果：SN 460/460、OpenAI 393/393，真实模型调用均为 0，实际费用均为 0 美元。
- Qwen Image 3 已改用官方异步 NativeTask：`qwen-image-3.0-pro` 的 `image.txt2img` 与 `image.img2img` 都通过异步提交及任务查询完成，不再占用 Zone Gateway 的 60 秒响应头窗口。相关 T1 为 3/3、Qwen 全量 T1.5 为 275/275，真实 T2 的两个目标单元均完成协议、任务和 2048×2048 PNG 交付校验；无 Judge 时 Runner 标记为人工复核，人工确认文生图正确生成测试数字 `4827`，图生图保留数字与主体构图并按要求改为暖色傍晚光照，因此合并结论为 2/2 通过。T2 共发起 3 次真实调用（含图生图前置图），无重试，未知费用估算敞口 0.03 美元，cleanup 已恢复临时凭据。

## 本轮 T1 报告

1. [T1 全矩阵报告：188 通过、1 失败](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T13-57-46-682Z-dd624b6f/summary.md)
2. [T1 失败项定向复验：1/1 通过](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T14-02-13-765Z-76ddd560/summary.md)
3. [Qwen Image 3 异步改造定向复验：3/3 通过](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T14-56-10-062Z-a1fb8df4/summary.md)

## 本轮 T1.5 报告

1. [SN 初次全量：299/451 通过](test/aicc_test/reports/acceptance/t15-20261018140456-3819810/summary.md)
2. [SN 第二次全量：344/460 通过](test/aicc_test/reports/acceptance/t15-20261018140858-3825383/summary.md)
3. [SN 最终全量：460/460 通过](test/aicc_test/reports/acceptance/t15-20261018141243-3830344/summary.md)
4. [OpenAI 受影响全量：393/393 通过](test/aicc_test/reports/acceptance/t15-20261018141529-3833843/summary.md)
5. [Qwen 异步协议全量：275/275 通过](test/aicc_test/reports/acceptance/t15-20261018145823-3894113/summary.md)

## 历史 T1 报告

1. [T1 全量报告 1](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T05-09-32-758Z-47402616/summary.md)
2. [T1 最终全量报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T06-27-49-158Z-dce1e035/summary.md)
3. [T1 修正后全量复验报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T08-21-04-357Z-ea94049f/summary.md)
4. [T1 资源分流修正后全量复验报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T13-02-00-973Z-fcb1ada8/summary.md)

## 历史 T1.5 报告

1. [T1.5 全量报告 1](test/aicc_test/reports/acceptance/t15-20261018051413-3065093/summary.md)
2. [T1.5 定向报告 1](test/aicc_test/reports/acceptance/t15-20261018054009-3100276/summary.md)
3. [T1.5 全量报告 2](test/aicc_test/reports/acceptance/t15-20261018054032-3100755/summary.md)
4. [T1.5 初始化报告](test/aicc_test/reports/acceptance/t15-20261018063240-3191502/summary.md)
5. [T1.5 全量报告 3](test/aicc_test/reports/acceptance/t15-20261018063257-3191936/summary.md)
6. [T1.5 定向报告 2](test/aicc_test/reports/acceptance/t15-20261018065517-3222081/summary.md)
7. [T1.5 最终全量报告](test/aicc_test/reports/acceptance/t15-20261018065609-3223222/summary.md)
8. [T1.5 Qwen/SN 修正后定向复验报告](test/aicc_test/reports/acceptance/t15-20261018082600-3352909/summary.md)
9. [T1.5 Qwen 资源分流修正后定向复验报告](test/aicc_test/reports/acceptance/t15-20261018130610-3732887/summary.md)

## 最新定向 T2 报告

1. [Qwen Image 3 异步定向实测：2 项交付成功、人工复核 2/2 通过](test/aicc_test/reports/acceptance/aicc-2026-10-18T15-05-43-442Z-1e4770de/summary.md)
2. [Qwen Image 3 图生图旧同步实现复测：0/1 通过](test/aicc_test/reports/acceptance/aicc-2026-10-18T14-27-33-200Z-2387a270/summary.md)

## 历史 T2 报告

1. [T2 组合计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-10-16-048Z-1222211d/summary.md)
2. [SN 初始计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-10-41-691Z-279cc3b4/summary.md)
3. [SN 全量计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-12-17-208Z-321e464b/summary.md)
4. [SN 全量实测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-12-32-686Z-f0b7be37/summary.md)
5. [受影响 Provider 矩阵计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-15-25-965Z-e0b366fe/summary.md)
6. [受影响 Provider 矩阵实测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-15-37-799Z-17c37b57/summary.md)
7. [GLM 与 Qwen Image 定向报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-17-28-063Z-1ea46d5b/summary.md)
8. [Qwen Image 3 定向报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T07-24-54-029Z-2621734c/summary.md)
9. [Qwen Image 3 图生图复测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T07-27-08-006Z-88c38b0e/summary.md)
