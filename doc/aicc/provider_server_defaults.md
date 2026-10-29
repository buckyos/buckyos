# Provider 接入与服务端默认参数

## 身份、预设和用途

协议兼容只决定如何编码请求，不赋予模型身份、能力、思考档位或逻辑用途。接入按以下顺序核验：模型发现、Model Driver 身份匹配、渠道预设映射、任务规格准入、权限与能力过滤、真实推理。

`.model.json` 记录原厂公开参数与规格；渠道支持的子集、参数转换和禁用的 API 放在 `.provider.json`。例如 Qwen Max 原厂支持 `none/low/medium/xhigh`，SN/OpenRouter 的强制思考渠道仅映射 `low/medium/xhigh`；`minimal/high` 的兼容别名不生成新的原厂预设。原厂参数调整后，引用模型的各渠道映射和 T1.5 协议预期须同步检查。

`llm.chat` 只显式引用批准的规格，包含 Kimi Flagship、Qwen Max；并非发现任意 LLM 就自动准入。每个模型独立构建库存进行回归，避免其他可用模型遮盖缺失映射。普通逻辑树 item 仍不得把基础 exact model 直接挂入 LLM 任务。

## 显式默认模式

自定义 Provider 向导的高级配置仅选择渠道预设。创建后可在详情的 Provider rules 页设置别名和服务端默认用途，也可导入 JSON 文件或内置规则。例如：

```json
{
  "provider_profile_id": "custom",
  "protocol_adapter_id": "openai-responses",
  "instance_rules": {
    "model_driver_overrides": {
      "my-model": "qwen/qwen3.7-plus"
    },
    "server_default_routes": {
      "my-model": ["llm.chat"]
    }
  }
}
```

`server_default_routes` 的键是端点原始模型 ID，值是已存在的 LLM 任务路径。它不接受通配符、未知任务、规格或家族路径。模型须已被识别并支持 LLM；缺少 LLM metadata 或仅供直连的规格不能通过默认模式绕过准入。实验模型、能力、地域、权限、健康度和额度检查继续生效。

默认模式将无 variant 的基础 exact model 显式加入所选用途，标记来源 `provider_default`。它不制造 `native`、`reasoning-none` 或其他 effort。执行时省略思考档位控制参数；不会把“使用端点默认行为”解释为“关闭思考”。调用者显式给出的其他 canonical 参数及协议必填字段仍被处理，渠道的 operation、请求转换和约束仍适用。该模式不是可复现的固定 effort 承诺。

选择显式 effort 的 exact model 继续要求当前渠道映射；未支持的 effort 仍拒绝。别名映射只解决身份，不能生成这些预设。默认模式与显式预设可以共存；列出的任务只使用该物理模型的基础候选，不再被同模型的预设候选抢占。其他任务和显式 exact effort 调用仍使用已配置预设。

## 管理和持久化

`provider.add/validate` 使用现有 `instance_rules`；`provider.update` 支持 `provider_rules_id`，空字符串清除所选渠道规则，省略保持原值。`provider.list` 返回渠道规则及实例规则，以便编辑回读。设置仍存于 `services/aicc/settings`，通过既有 revision CAS 更新；库存快照保存默认用途并沿现有 LKGS 加载链恢复，不新增存储服务或文件路径。

`provider.validate` 返回模型发现、`models_recognized`、`unmatched_models`、`unavailable_presets`、`routing_candidates` 和 `warnings`。候选来自该实例库存与内置用途树的配置检查，不代表当前租户策略放行，也不证明上游支持真实推理。未知渠道规则、无效任务和模型身份问题应分别定位；余额能力也不能仅由模型发现成功推断。

## 验收与关闭条件

默认模式先验证无 variant 的基础调用，再核对 `llm.chat` 候选和最终请求不注入思考档位；显式 effort 另测正确编码和不支持档位拒绝。覆盖库存持久化恢复、任务边界和单模型完整路由。

配置测试、Mock 和模型列表均不足以关闭接入问题。关闭前须完成授权范围内的真实 SN 推理，以及无本地路由覆盖的 MessageHub → Jarvis → AICC → Provider → 消息回复链路，确认实际 Provider/模型归因、非空回答和清理结果。若账户、上游或运行环境阻塞，应保留问题未验收状态并记录证据，不把配置成功写成对话成功。

本分支不跟踪运行报告。测试明细仅保留在本地忽略的报告目录，供 review 使用。

参数核对来源：[Qwen 原厂 Chat 参数](https://help.aliyun.com/en/model-studio/qwen-api-via-openai-chat-completions)、[OpenRouter 对应快照](https://openrouter.ai/qwen/qwen3.8-max-0902)、[GLM 5.3 原厂说明](https://docs.z.ai/guides/llm/glm-5.3)。

实现涉及既有公共路由和 lowering：为显式默认模式增加 task 准入、同模型预设过滤及思考参数省略；不能仅改某个 Provider codec，否则默认模式无法进入逻辑用途或会被其他预设抢占。管理字段同步 Rust API、WebSDK 和 Desktop，不增加第三方依赖。媒体调用协议未改变，`aigc_image`、`aigc_audio`、`aigc_video` 无需添加模型说明；`buckyos_admin` 仅补通用接入诊断。
