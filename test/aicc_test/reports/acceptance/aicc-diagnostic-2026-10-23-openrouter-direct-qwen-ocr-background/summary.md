# OpenRouter 直连 Qwen OCR background 诊断

日期：2026-10-23（Asia/Shanghai）。按用户明确要求直接调用 OpenRouter，绕过 SN、AICC 和 Zone Gateway；没有修改运行配置或任何 provider 实现。

| 请求 | 参数 | 结果 | 耗时 |
|---|---|---|---|
| POST https://openrouter.ai/api/v1/responses | qwen/qwen3.7-plus，background=true，stream=false，max_output_tokens=4096；原 image_ocr.png 的 base64 输入 | HTTP 401，error.message=`User not found.`，code=401 | 响应头及完整响应均约 807 ms |
| GET https://openrouter.ai/api/v1/key（只读核实） | 相同凭据 | HTTP 401，error.message=`User not found.` | 未记录 |

图片为原 OCR 输入：2189 bytes，SHA256 eaa912debbdd30ccbbebbdae7ac4950b321afaaf740dd98ad76af3d7cfdc19c9；提示词保持 `Extract all visible text from this document.`。没有添加 store 参数，没有改变 reasoning 设置。

本次鉴权未通过，没有取得模型输出、usage、排队状态或后台任务 ID，未执行任务查询。因此不能用此结果比较 SN 与 OpenRouter 的延时，不能判断 background 的长任务支持。需要可用的 OpenRouter 凭据才能继续。公开模型目录 GET 返回成功不代表凭据有效，已由专用 key 查询核实。

只发出一次推理端点请求，未重试；另有模型目录与 key 信息的只读查询。凭据仅保存在本地被 Git 忽略的权限 0600 TOML 文件中，报告不含凭据；没有生成媒体或外部消息，没有临时实例或网关变更需要恢复。
