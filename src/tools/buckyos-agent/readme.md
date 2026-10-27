# BuckyOS Agent

> 待buckyos-web sdk文档后，基于TS 实现

buckyos-agent tools 是用 TypeScript 编写、运行在 Deno 环境下的 BuckyOS 命令行工具。
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
2) 按 SDK 类型构造 canonical method request
3) initRuntime() 拿到登录后的 BuckyOS 会话
4) 逻辑模型先 route.resolve，再调用 typed inference；文生图直接调用 Helper
5) 把产物交到本 Agent 的 artifacts 桶（命令返回 obj://bucket/...；给了输出路径才另存本地），或读取文本
6) emit() 一行 AgentToolResult JSON 给上游
```

**`gen_image.ts` 是带详细注释的范本**，其他命令文件用同样的骨架。改变化点
只在 (1) (2) (5)；(3)(4)(6) 是机械的。

`lib/` 是这套工具的"标准库"，Agent 写自己的新命令时同样从这里拿原件：

| 文件 | 用途 |
|---|---|
| `lib/runtime.ts` | `initRuntime()` — 登录 BuckyOS |
| `lib/aicc.ts` | `callAicc()` canonical 路由与 typed inference + task-manager 轮询；`textToImage()`、`commonPolicyOptions()`、`describeFailure()` |
| `lib/io.ts` | `resolveInputResource()` 把字符串映射成 ResourceRef；`pickArtifact()`；`deliverArtifact()` 把产物交到 artifacts 桶（可选另存本地）；`saveResourceToPath()` / `saveBucketObjectToPath()` 落盘；`suffixPathByMime()` |
| `lib/bucket.ts` | 本 Agent 的桶：`uploadNamed()`（字节直传 `artifacts` 或 `scratch`，≤32 MiB 走 `PUT /bucket/v1`，更大走 NDM 上传 + `bind`）、`copyToArtifacts()`（`bucket.copy`）、`readBucketObject()`、`obj://bucket/...` 解析 |
| `lib/cli.ts` | `parseArgvOrExit()`、`requireString/flagInt/flagFloat/flagBool`、`COMMON_OPTIONS_HELP`、`bailArgError()` |
| `lib/result.ts` | AgentToolResult builders + `emit/emitAndExit`、各种 `bail*` 错误收敛、退出码常量 |
| `lib/types.ts` | Agent 输出类型，以及从发布版 `buckyos` SDK 派生的 AICC response / `ResourceRef` 视图 |

写一个新命令的最短模板：复制 `gen_image.ts`，把 `TOOL` / `METHOD` /
`capability` / `HELP` / step 2 的 typed request builder / step 5 的产物处理换成
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

AIOS 镜像中的只读安装目录是 `/opt/buckyos/bin/opendan/buckyos-agent/`。镜像不带 SDK：
安装态 `deno.json` 的 `buckyos` 指向 `/opt/buckyos/libexec/buckyos-tool/dist/node.mjs`，即
node-daemon 从宿主 `$BUCKYOS_ROOT/libexec/buckyos-tool/` 只读挂入 Agent 容器的 SDK，
与容器内 `buckyos` 命令用的是同一版（随宿主系统升级）。该挂载缺失时 `aicc-tool` 及各命令直接报错退出。
仓库内开发态 `deno.json` 指向构建产物 `src/rootfs/libexec/buckyos-tool/dist/node.mjs`。
在 Agent 容器内可以直接运行 task：

```bash
cd /opt/buckyos/bin/opendan/buckyos-agent
deno task gen_image --help
```

Agent 组合新工具时，可以读取该目录下的 `commands/*.ts` 和 `lib/*.ts`，SDK 类型见
`/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts`，但应把派生脚本写入 Agent 或当前 Session
自己的 `tools/` 目录，不要修改只读安装副本。运行派生脚本时显式复用安装态配置：

```bash
deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json \
  --allow-net --allow-read --allow-write --allow-env \
  --unsafely-ignore-certificate-errors \
  ./tools/my-aicc-tool.ts
```

### 命令清单

`gen_image` / `edit_image` / `inpaint_image` / `upscale_image` / `remove_bg`
`ocr_image` / `caption_image` / `detect_image` / `segment_image`
`text_to_speech` / `speech_to_text` / `gen_music` / `enhance_audio`
`gen_video` / `edit_video` / `extend_video` / `upscale_video`
`ai_provider list|health` / `ai_quota` / `materialize_resource`

每个命令的 `--help` 列出参数。

`gen_video` 对应统一的 `video.generate`（协议见 `doc/aicc/video_generation.md`）：文本、首尾帧
（`--first-frame/--last-frame`）、参考图/视频/音频（可重复的 `--ref-image/--ref-video/--ref-audio <id>=<素材>`，
`--clip`、`--with-audio` 细化参考片段）都是同一请求的 `conditions`，提示词用 `@{id}` 引用素材；输出规格
（`--duration/--aspect/--short-edge|--size/--fps/--audio/--format`）进入 `output`。逻辑模型走
`helper.video_generate`，由 AICC 按实测素材选模；模型不支持的条件或规格直接报错，不会被丢弃。
`edit_video` 对应 `video.edit`。视频结果解析 `videos[]`，多于一个视频时报错而不是截断，
`detail.video` 带 Provider 声明值、AICC 实测值和偏差。

### 输出协议

CLI 始终将 `AgentToolResult` JSON 写到 stdout（与 `src/frame/agent_tool` 的协议一致，`agent_tool_protocol: "1"`），stderr 用于进度日志。退出码遵循 doc §2.5：

异步 AICC 任务等待期间，CLI 每 5 秒向 stderr 写一行以
`__BUCKYOS_AGENT_PROGRESS__` 开头的结构化心跳。OpenDAN 会消费该心跳以更新会话状态并续租 `shell` 的空闲超时；它不属于最终 `AgentToolResult`。

- `0` 成功
- `1` 参数错误
- `2` AICC 调用失败
- `3` provider/route 失败
- `4` 等待超时
- `5` 本地 I/O 失败

### 输入/输出处理

- 输入文件：本地路径 → base64；`http(s)://...` → URL；`named_object:<obj_id>` → NamedObject；`obj://bucket/<bucket_id>/<key>` → 它绑定的 NamedObject
- 输出 artifact：一律进本 Agent 的 `artifacts` 桶（`user/<agent app>/<owner>/artifacts`，`documents` 档，归用户、备份），结果 `detail.files[].url` 是 `obj://bucket/...`，直接列进报告的 artifacts。AICC 已存进它的 `outputs` 桶（`user/aicc/<owner>/outputs`，7 天）的结果用 `bucket.copy` 复制过来，不再传字节；URL/base64/raw PCM 读出字节后上传一次
- 中间文件：`uploadNamed(runtime, bytes, name, { bucket: "scratch" })` 进 `scratch` 桶（`temp`，7 天）
- 资源落盘：`materialize_resource obj://bucket/... | named_object:<obj_id> <output_path>`（本地工具需要路径时的显式降级）；目标文件已存在时拒绝覆盖

### 环境变量

在 OpenDAN 中，工具使用运行时注入的受限 AppClient session token，并通过宿主机 NodeGateway 访问系统服务。本地独立运行时仍需私钥（默认搜索路径 `~/.buckyos`、`/opt/buckyos/etc` 等）。容器内 `buckyos` 命令另读 node-daemon 注入的 `BUCKYOS_TOOL_ENDPOINT`（见 `doc/arch/11_env_contract.md`），本工具不读该变量。

- `BUCKYOS_APP_ID`（默认 `buckyos-agent`，兼容 `BUCKYOS_TEST_APP_ID`）
- `BUCKYOS_APPCLIENT_SESSION_TOKEN`：OpenDAN 注入的 AppClient session token；设置后启用容器内访问方式
- `BUCKYOS_HOST_GATEWAY`：容器访问宿主机的地址（Docker 默认 `host.docker.internal`，Windows WSLC 由 node-daemon 注入 `host.wslc.internal`）
- `BUCKYOS_NODE_GATEWAY_PORT`：NodeGateway 端口（默认 `3180`）
- `BUCKYOS_AICC_TOOL_ROOT`：仅供 AIOS launcher 调试覆盖安装目录（默认 `/opt/buckyos/bin/opendan/buckyos-agent`）
- `BUCKYOS_OWNER_USER_ID`：当前 Zone owner 用户 ID（默认 `devtest`）
- `BUCKYOS_ZONE_HOST`（默认 `test.buckyos.io`，兼容 `BUCKYOS_TEST_ZONE_HOST`）
- `BUCKYOS_APP_CLIENT_DIR` / `BUCKYOS_TEST_APP_CLIENT_DIR`：额外私钥搜索目录
- `AICC_DEFAULT_PROFILE`：`balanced|cheap|fast|quality`（默认 `balanced`）
- `AICC_DEFAULT_TIMEOUT`：等待 task 完成的毫秒上限（默认 `900000`）
