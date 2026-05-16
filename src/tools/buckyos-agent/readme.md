# BuckyOS Agent

> 待buckyos-web sdk文档后，基于TS 实现

buckyos-agent tools 是用ts编写的，运行在deno环境下的buckycli工具。
会默认打入opendan的docker镜像(paios/aios), 是opendan agent runtime为Agent提供的，访问buckyos的基础工具
是用ts编写是方便开放源代码给Agent,Agent可以在此代码基础上，自行升级合组合

## MVP功能需求

1. 通过control_panel的接口发布static-web-app
应为jarvis的权限问题，只能执行到store,需要传一个页面url让用户自己点开，确认上线
发布到哪里的问题？
- 一个通用的publish目录，放在里面的所有文件夹立刻就能得到预览
- 打包成app发布，需要用户手工鉴权

2. aicc命令组

## AICC Agent CLI Tools

实现见 `aicc-tool.ts` + `commands/*.ts`，需求文档：[`doc/aicc/aicc_agent_cli_tools.md`](../../../doc/aicc/aicc_agent_cli_tools.md)。

### 给 Agent: 为什么源码长这样

Agent 应当能直接读 `commands/*.ts` 学到怎么调 AICC，然后自己写新的 deno
工具（这比把若干 CLI 用管道串起来更可控）。所以每个命令文件刻意写成一份
**从上到下的配方**，固定 6 步：

```
1) 解析 argv → 位置参数 + 选项
2) 构造 input_json（AICC payload.input_json 的字段）
3) initRuntime() 拿到登录后的 BuckyOS 会话
4) callAicc() 同步调用 + 等待 task 完成（如果异步）
5) 从 AiResponse.message 的派生视图把产物拉回本地（或读取文本）
6) emit() 一行 AgentToolResult JSON 给上游
```

**`gen_image.ts` 是带详细注释的范本**，其他命令文件用同样的骨架。改变化点
只在 (1) (2) (5)；(3)(4)(6) 是机械的。

`lib/` 是这套工具的"标准库"，Agent 写自己的新命令时同样从这里拿原件：

| 文件 | 用途 |
|---|---|
| `lib/runtime.ts` | `initRuntime()` — 登录 BuckyOS |
| `lib/aicc.ts` | `callAicc()` 同步调用 + task-manager 轮询；`commonPolicyOptions()`、`requestNamedObjectOutput()`、`describeFailure()` |
| `lib/io.ts` | `resolveInputResource()` 把字符串映射成 ResourceRef；`pickArtifact()`、`saveArtifactToPath()` 落盘 artifact；`suffixPathByMime()` |
| `lib/cli.ts` | `parseArgvOrExit()`、`requireString/flagInt/flagFloat/flagBool`、`COMMON_OPTIONS_HELP`、`bailArgError()` |
| `lib/result.ts` | AgentToolResult builders + `emit/emitAndExit`、各种 `bail*` 错误收敛、退出码常量 |
| `lib/types.ts` | 协议类型（`AgentToolResult`、`AiResponse`、`ResourceRef`、`Capability`、`Profile` 等）|

写一个新命令的最短模板：复制 `gen_image.ts`，把 `TOOL` / `METHOD` /
`capability` / `HELP` / step 2 的 input_json builder / step 5 的产物处理换成
你要的就行。


### 调用方式

```bash
# 单一 binary + 子命令
deno run --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors aicc-tool.ts <command> [args...]

# 或 deno task
deno task gen_image "A matte black desk lamp" result.png --quality high

# 也可以把 aicc-tool 当 binary 安装后，给每个 command 建 symlink
# (file 名匹配 commands/*.ts 文件名时，自动作为子命令)：
ln -s aicc-tool gen_image
gen_image "prompt" out.png
```

### 命令清单

`gen_image` / `edit_image` / `inpaint_image` / `upscale_image` / `remove_bg`
`ocr_image` / `caption_image` / `detect_image` / `segment_image`
`text_to_speech` / `speech_to_text` / `gen_music` / `enhance_audio`
`gen_video` / `img2video` / `video2video` / `extend_video` / `upscale_video`
`ai_provider list|health` / `ai_quota`

每个命令的 `--help` 列出参数。

### 输出协议

CLI 始终将 `AgentToolResult` JSON 写到 stdout（与 `src/frame/agent_tool` 的协议一致，`agent_tool_protocol: "1"`），stderr 用于进度日志。退出码遵循 doc §2.5：

- `0` 成功
- `1` 参数错误
- `2` AICC 调用失败
- `3` provider/route 失败
- `4` 等待超时
- `5` 本地 I/O 失败

### 输入/输出处理

- 输入文件：本地路径 → base64；`http(s)://...` → URL；`named_object:<obj_id>` → NamedObject
- 输出 artifact：`named_object` 走 `ndm_proxy.openReader` 拉取并落盘；URL/base64 直接写文件

### 环境变量

继承 `test/aicc_test` 的 BuckyOS 接入方式，所以本地需要私钥（默认搜索路径 `~/.buckyos`、`/opt/buckyos/etc` 等）。

- `BUCKYOS_APP_ID`（默认 `buckyos-agent`，兼容 `BUCKYOS_TEST_APP_ID`）
- `BUCKYOS_ZONE_HOST`（默认 `test.buckyos.io`，兼容 `BUCKYOS_TEST_ZONE_HOST`）
- `BUCKYOS_APP_CLIENT_DIR` / `BUCKYOS_TEST_APP_CLIENT_DIR`：额外私钥搜索目录
- `AICC_DEFAULT_PROFILE`：`balanced|cheap|fast|quality`（默认 `balanced`）
- `AICC_DEFAULT_TIMEOUT`：等待 task 完成的毫秒上限（默认 `180000`）
