# SN 禁用策略的运行时库存验证

通过真实 Zone Gateway 登录后读取 T1.5 临时 SN Provider 库存，没有真实模型调用。该实例由 SN discovery Mock 提供包含所有目标模型的目录，使用与真实 SN 相同的 Provider Rules 和 inventory 构建逻辑。

- SN provider metadata revision：15；已安装到 `/opt/buckyos/etc/aicc/driver_metadata/local/providers/sn.provider.json`。
- `qwen-image-3-pro`、`wan-3.0` 均未出现在运行时库存。
- Grok、Terra、Qwen3.7 Plus 及其可用推理预设仍可见。
- 验证前原系统没有常驻 SN 模型库存；本次在受控临时 SN 实例中验证排除规则，结束后移除该实例。永久生效的是 local Provider metadata override。

| 实例 | 库存条目数（含预设） | 基础模型 | API types |
|---|---:|---|---|
| `t15-20261109081136-2942440-sn-ai-provider` | 15 | gpt-6.1-terra | agent.computer_use, image.img2img, image.txt2img, llm, vision.caption, vision.ocr |
| `t15-20261109081136-2942440-sn-ai-provider` | 15 | grok-4.7 | llm, vision.caption, vision.ocr |
| `t15-20261109081136-2942440-sn-ai-provider` | 15 | qwen3.7-plus | llm, vision.caption, vision.ocr |

Rust InventoryBuilder 回归也明确提供两个被禁用模型作为发现输入，断言它们被排除、Qwen 原厂 API 能力仍保留。此次只有测试断言和 JSON 配置变更，没有公共运行组件的逻辑修改。
