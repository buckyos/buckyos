# AICC Gemini T1/T1.5/T2 修复记录（2026-10-14）

## 问题原因

- Gemini 本地模型清单和能力基线落后于官方 Models API 与模型文档，导致 T2 官方清单对账失败。
- Gemini 视觉检测/分割仍按旧的自定义结果形状解析，没有完整接受官方 `boxes[].box_2d/mask/label`，也不能兼容真实 2.5 模型返回的 fenced JSON、顶层单对象和扁平 polygon。
- `embedding.text` 使用单请求 `embedContent`，但 canonical API 一次允许多个 item；官方对应接口是 `batchEmbedContents`。
- Interactions 视频输出的 `response_format.duration` 被编码为 JSON number，官方协议要求秒数字符串。
- T2 长轮次结束后继续用最初的应用会话检查运行时清理，会在 JWT 过期时使已经完成的整轮无法归档。
- Fal rembg 队列生命周期 URL 断言没有覆盖多段路径。

## 修复方式

- 更新 Gemini Provider/Model metadata、T2 capability baseline 和 route exposure contract，以官方模型清单、生命周期和能力为准。
- 仅修改 Gemini 私有协议实现：
  - 视觉请求使用官方结构化输出 schema，并归一化官方及已观察到的旧模型兼容响应。
  - `embedding.text` 固定使用 `models.batchEmbedContents`；`embedding.multimodal` 保持 `models.embedContent`。
  - 视频时长编码为 `7s` 形式。
- 同步增强 T1.5 Gemini Mock：独立声明并验证 batch embedding 契约、官方视觉 schema 和视频时长格式，确保真实 Provider 暴露的问题能在 T1.5 发现。
- T2 收尾检查在有用户名/密码时重新登录，避免长轮次因初始 JWT 过期丢失报告。
- 扩充 Provider restriction 分类，并让结构化响应诊断保留脱敏片段。
- 修正 Fal rembg 多段生命周期 URL 的测试覆盖。

没有修改 `buckyos-api`、调度、存储或其他基础公共组件，也没有新增依赖。

## 验证

- T1：189/189。
- T1.5 全 Provider：1806/1806；更新后的 Gemini 全量：414/414。
- Gemini 协议单元测试：22/22。
- AICC acceptance self-test：119/119。
- T2 最新矩阵预检：123 个用例、0 个 capability mismatch。
- 修复后的真实 Gemini text embedding：2/2。
- 检测/分割最终 T1.5：2/2；T2 共 28 个模型/API 组合，其中 23 个完成调用并进入 review、4 个 provider_restricted、1 个因网关响应头超时失败。
- `uv run buckyos-build.py -s aicc` 成功，部署后二进制哈希一致。
- `cargo fmt --check -p aicc`、`git diff --check` 通过。

## 未闭环项

- 7 个媒体输入 URL 被 Gemini 判定为私网/排除 IP；当前 NDN 公网入口未能向 Provider 提供可抓取文件。
- 3 个超过 60 秒的同步调用被 cyfs-gateway 的 response-head timeout 转为 500；未修改网关公共逻辑。
- Gemini 官方 `background` 交互会把 `gemini-3.1-pro-preview-customtools` 路由到不支持图像输入的 `-agent` 变体，不能作为视觉同步超时的通用替代方案。
- 两个 Veo extension 请求被 Provider 以 `durationSeconds` 越界拒绝；同模型的 text/image-to-video 已完成真实调用。
