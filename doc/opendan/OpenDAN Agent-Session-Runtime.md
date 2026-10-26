# OpenDAN Agent Session Runtime

本文定义 OpenDAN Session 的两种环境基线：标准环境（`standard`）与已扩展环境（`extended`），重点说明命令、运行依赖、环境声明与检查的关系。内容依据 2026-10-25 的仓库实现；命令清单以源码和对应安装版本为准。

## 1. 两种环境的关系

**已扩展环境是标准环境的超集合。** 任何声明为 `extended` 的 session-runtime 都应满足标准环境的全部要求，并在此基础上提供已经准备好的额外工具。

```text
standard = 核心 Agent 命令 + Agent/Session 上下文 + BuckyOS 服务调用能力
extended = standard + 该 runtime 已准备的第三方工具、语言环境或构建工具链
```

| Session 环境 | 要求 standard 的 cmd_skill | 要求 extended 的 cmd_skill |
| --- | --- | --- |
| `standard` | 满足基线要求 | 不满足基线要求 |
| `extended` | 满足基线要求 | 满足基线要求，仍须检查具体工具及平台 |

这也是 [`RuntimeBaseline::meets`](../../src/kernel/buckyos-api/src/app_schema.rs) 的匹配规则。`extended` 不替换、删减或改变标准命令的语义。

两种基线都描述 **Session 执行端具备什么能力**，与执行端的位置和操作系统分别记录：

- `container`、`host_native`、`remote_node` 等说明执行位置；`standard`、`extended` 说明环境基线。节点上的 runtime 同样可以是 `extended`。
- `platform` 说明操作系统、架构和 shell。`extended` 不会让 Windows 自动具备 Linux 命令或脚本语法。
- Workspace 说明材料和项目文件所在的 runtime 与目录，不决定 Session 的基线。只用来存放 Workspace 的机器不必安装完整 Agent 工具集。
- 一个 Session 创建时冻结自己的 runtime、基线和平台。选择顺序为显式 `--runtime`、继承父 Session、Agent 默认；修改默认 runtime 或其声明不会自动改变已有 Session。

当前系统对 aios 容器声明 `extended`；host/节点 runtime 取 owner 的声明，缺省 `standard`。**声明为 extended 不表示装齐所有开发软件，也不表示必须挂载 ExtTool 卷。** 当前 aios 的实际附加工具见 §3。

## 2. 标准环境要求

### 2.1 执行和运行依赖

标准环境应能在 Session 的实际执行端完成以下工作；控制侧机器安装了工具，不能替代执行端的准备。

| 项目 | 要求 |
| --- | --- |
| 命令执行 | 有可用的 shell，正确传递参数、stdin、stdout、stderr 与退出码。当前 Node Executor 在 Unix 默认使用 Bash，在 Windows 优先使用 `pwsh.exe`，否则使用 Windows PowerShell。命令应按目标 shell 的语法书写。 |
| 命令发现 | §2.2 的产品命令在 Session 的执行环境中可发现、可启动；Windows 可使用对应 `.exe` 等平台入口。不能只在用户交互终端的 PATH 中可见。 |
| 工作目录 | 有可读写的 Session 工作目录，作为默认 cwd 和产物位置。绑定 Workspace 不会改变默认 cwd；跨 runtime 的文件通过 `agent-session copy` 交接。 |
| Session 上下文 | `agent-session` 等命令能确定所属 Agent、Session 和调用上下文，访问对应的 AgentService。远端执行不要求能直接读取控制侧 AgentRoot。 |
| BuckyOS 身份 | `buckyos`、AICC 命令和 `agent_tool xllm` 能取得该 Agent 的服务调用身份，并连接目标 Zone 的服务。具体操作仍受身份权限约束。 |
| TS 工具运行依赖 | 安装匹配的 buckyos-agent 源码、`deno.json`、BuckyOS Tool/SDK 与 Deno 运行时，使 AICC 命令及 `buckyos` 启动链完整。只创建命令链接不够。 |
| 长任务 | 执行器能返回任务状态和结果，按其能力提供查询、等待与取消；恢复时继续使用原来的 runtime。标准环境不要求安装 tmux，也不保证两次 shell 调用之间保留 `cd` 或临时环境变量。 |

Deno 是当前标准 TS 工具的运行依赖，但 `deno` 这个命令名不在标准命令清单中：host 可以由 launcher 使用 Tool 自带 Deno 的绝对路径。`python3`、`node`、`git`、`jq`、`ffmpeg` 等也不能仅凭 `standard` 推定存在。

### 2.2 标准命令清单

唯一的机器可读清单是 [`libopendan::baseline::STANDARD_TOOLS`](../../src/frame/lib_opendan/src/baseline.rs)。镜像构建、host 入口安装与 runtime 基线检查复用这份清单。

以下按用途列全当前清单的 28 个入口。其中 `ai_provider_test` 是当前实现误收的测试入口，情况见 §2.3；其余 27 个为产品命令。命令参数以安装版本的 `--help` 为准。

#### Agent 与 Session

| 命令 | 用途 |
| --- | --- |
| `agent_tool` | Agent 通用工具入口；其中 `agent_tool xllm` 推进 LLM context。基线检查的是 `agent_tool` 可执行入口，不是要求另有名为 `xllm` 的独立命令。 |
| `agent-session` | 当前 Session 的操作入口，包括 Session/Goal/Todo、runtime、Workspace、skill、task 和文件复制等操作；具体操作受上下文和权限限制。 |
| `xagent` | Session 创建、驱动、查询及 Agent State 命令行入口，也提供 `runtime baseline` 清单和检查。 |
| `llm_understand_media` | 通过模型理解图片、音频、视频等材料。 |

#### BuckyOS 与 AICC 服务

| 命令 | 用途 |
| --- | --- |
| `buckyos` | BuckyOS 系统、App、文件、AIWorkspace、Agent/skill 等 CLI 能力的统一入口。 |
| `aicc-tool` | AICC 配方命令的统一分派入口，如 `aicc-tool gen_image …`；可分派项以其 `--help` 为准。 |
| `ai_provider` | 查询 provider、模型清单及模型健康状态；子命令包括 `list`、`models`、`health`。 |
| `ai_quota` | 查询影响当前身份或指定 App 的 AI 配额。 |
| `materialize_resource` | 将对象、URL 等资源保存为当前执行端的本地文件。 |
| `resume_aicc_task` | 查询/继续等待已有 AICC 任务并取得结果，避免重复提交生成请求。 |
| `ai_provider_test` | 当前清单中的测试入口；不是可用的产品 CLI，不应用它做 provider 健康检查，见 §2.3。 |

#### 图像

| 命令 | 用途 |
| --- | --- |
| `gen_image` | 图像生成。 |
| `edit_image` | 按指令编辑图像。 |
| `inpaint_image` | 图像局部重绘。 |
| `upscale_image` | 图像放大、增强。 |
| `remove_bg` | 移除图像背景。 |
| `ocr_image` | 通过 AICC 识别图像文字。 |
| `caption_image` | 生成图像描述。 |
| `detect_image` | 图像目标检测。 |
| `segment_image` | 图像分割。 |

#### 音频

| 命令 | 用途 |
| --- | --- |
| `text_to_speech` | 文本转语音。 |
| `speech_to_text` | 语音转文本。 |
| `gen_music` | 音乐生成。 |
| `enhance_audio` | 音频增强。 |

#### 视频

| 命令 | 用途 |
| --- | --- |
| `gen_video` | 视频生成。 |
| `edit_video` | 按指令编辑视频。 |
| `extend_video` | 请求视频延展；实际支持受 provider 能力和输入条件限制。 |
| `upscale_video` | 视频放大、增强。 |

**音视频的 AICC 命令属于标准环境；本地音视频加工软件属于扩展工具。** 例如 `gen_video` 在标准清单中，而 `ffmpeg`、`ffprobe` 不在。`aigc_audio` 和 `aigc_video` 均声明 `baseline: standard`，可在标准环境调用 Zone 模型生成音视频；剪辑、合成及本地媒体检查步骤再核验所需的 `ffmpeg`、`ffprobe` 或 ImageMagick。缺少这些加工工具不表示 Zone 没有生成模型。

命令可启动表示执行端有对应接口，不保证当前 Zone 已配置所需 provider、模型、额度和资源访问权限。模型不支持某操作、额度不足或服务暂时不可达，都应报告实际结果，不据此修改环境基线。

### 2.3 当前标准清单的实现注意项

当前 [`commands/ai_provider_test.ts`](../../src/tools/buckyos-agent/commands/ai_provider_test.ts) 是 `Deno.test` 测试文件，却被收入 `STANDARD_TOOLS`，镜像也会为 `commands/*.ts` 创建入口；实际 [`aicc-tool.ts`](../../src/tools/buckyos-agent/aicc-tool.ts) 的分派表不接受 `ai_provider_test`。因此存在这个入口不代表它是可用的产品命令。provider 查询应使用 `ai_provider`。

本文保留该项以便和当前自动检查结果对照，不把测试文件提升为环境设计要求。后续维护清单时，应同步修正源码枚举、打包和本文。

### 2.4 当前安装布局

| 内容 | aios 容器 | host/节点 |
| --- | --- | --- |
| Agent 核心程序 | `/opt/buckyos/bin/opendan/` 下的程序及多入口；Session helper 由 `.runtime/bin` 装配 | `$BUCKYOS_ROOT/bin/agent-tools/` 下指向 `opendan` 的多入口；节点执行器将该目录前置到 PATH |
| buckyos-agent 源码 | `/opt/buckyos/bin/opendan/buckyos-agent/` | `$BUCKYOS_ROOT/libexec/buckyos-agent/` |
| AICC 命令入口 | `/usr/local/bin/aicc-tool` 及各命令链接 | `bin/agent-tools/` 内的同名入口，启动 buckyos-agent |
| BuckyOS Tool/SDK | 宿主只读挂载到 `/opt/buckyos/libexec/buckyos-tool/`，由容器 launcher 调用 | `$BUCKYOS_ROOT/libexec/buckyos-tool/`，系统 CLI 为 `bin/buckyos` 或 `bin/buckyos.cmd` |
| Deno | 镜像内 Deno | `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`，Windows 为 `deno.exe` |
| Session 上下文 | 通过 Session 环境和 AgentRoot 访问 | libopendan 在 Session 工作目录写入 `.opendan/session.json` 与 `.opendan/session.token`，helper 经 kRPC 访问 AgentService |

host 入口由 `opendan install-agent-tools` 布置：Unix 使用符号链接，Windows 使用硬链接或复制。Node Executor 不依靠复制控制侧的 `OPENDAN_*` 环境来获得 Session 身份。机制见 [Agent State Protocol](<protocol/Agent State Protocol.md>) 和 [OpenDAN README](../../src/frame/opendan/README.md)。

## 3. 已扩展环境要求

### 3.1 扩展工具按实际能力准备

`extended` 必须保留 §2 的全部能力，然后为目标任务准备额外工具。当前实现没有与 `STANDARD_TOOLS` 对应的统一 `EXTENDED_TOOLS` 清单，也没有“声明 extended 即安装工具”的流程。

准备扩展环境时，应明确提供哪些工具、版本、执行路径和任务能力；cmd_skill 用 `baseline: extended` 表示需要扩展环境，并在 `requires` 或正文中列出具体依赖。运行任务前仍需核验这些依赖。音频环境可以有 `ffmpeg` 而没有 Rust；开发环境可以有 Rust 而没有 ImageMagick，这两者都不能只凭 `extended` 声明运行对方的任务。

### 3.2 当前 aios 提供的附加工具

下面是当前 [`build_aios`](../../build_aios) 明确安装的软件及常用入口。它描述默认 aios 的附加能力，不是对所有 Windows、macOS、Linux 扩展环境强制要求同一套软件。

| 能力 | 常用命令 | 当前 aios 的来源与边界 |
| --- | --- | --- |
| Shell | `bash` | 镜像安装；host 使用其目标平台的 shell。 |
| 网络与版本管理 | `curl`、`git` | 镜像安装；远端服务身份和仓库访问权限另行检查。 |
| JSON、目录与文件传输 | `jq`、`tree`、`rsync` | 镜像安装。 |
| 解包、压缩 | `unzip`、`xz` | 镜像安装 `unzip`、`xz-utils`。 |
| Python | `python3`、`uv` | Python 解释器及依赖管理；项目依赖仍需按项目安装，不保证 `python` 或 `pip` 别名。 |
| JavaScript | `node`、`npm`、`npx` | Node 分发包；不因此保证 `pnpm` 或任意 npm 工具已安装。 |
| TypeScript | `deno` | 镜像安装，并用于标准 AICC 工具和脚本；项目模块及权限按脚本声明。 |
| 音视频加工 | `ffmpeg`、`ffprobe` | 镜像安装 ffmpeg；用于转码、裁剪、拼接、合成和媒体信息检查。 |
| 图像加工 | ImageMagick：`convert`、`identify`、`compare` | 镜像安装 ImageMagick；其它平台可能使用 `magick` 入口，应按实际版本调整调用。 |
| PDF 处理 | Poppler：`pdftotext`、`pdftoppm`、`pdfinfo` 等 | 镜像安装 `poppler-utils`；这些是包的常用入口，标准基线检查不逐项验证它们。 |
| 本地 OCR | `tesseract` | 镜像安装 `tesseract-ocr`；任务语言所需的数据包须单独确认。 |

以上是构建配置提供的软件范围，不是所有编解码器、字体、语言包或插件均已具备的保证。例如中文字幕需要可用的中文字库，Python 项目需要自己的依赖环境。软件版本以 `build_aios` 和实际运行端的版本输出为准。

### 3.3 ExtTool 提供的工具

ExtTool 是额外工具的分发方式之一。容器通过只读卷获得 `/opt/buckyos/tools/`，入口脚本将其 `bin/` 加入 PATH；它不改变 `extended ⊇ standard` 的关系，也不代替标准 Agent 命令的安装。

| 工具 | 常用入口 | 可用条件 |
| --- | --- | --- |
| Pandoc | `pandoc`、`pandoc-lua`、`pandoc-server` | 当前默认 ExtTool 构建安装这些入口；需要实际挂载对应版本的工具卷。Pandoc 不直接装在 aios 基础镜像内。 |
| FreeCAD CLI | `freecadcmd` | 构建 ExtTool 时提供 FreeCAD AppImage 才有真实程序。默认只有返回错误的占位入口，不能因 `command -v freecadcmd` 成功就认为 FreeCAD 可用。 |
| Python/Deno 依赖缓存 | 没有统一的业务命令入口 | ExtTool 预热下载缓存，不能据此认定依赖已安装进每个 Session 的解释器或项目环境。 |

来源：[ExtTool Dockerfile](../../publish/exttool/Dockerfile)、[Pandoc 安装脚本](../../publish/exttool/install_pandoc.sh)、[FreeCAD 安装脚本](../../publish/exttool/install_freecad.sh)、[容器入口脚本](../../publish/aios/entrypoint.sh)。

### 3.4 开发与构建任务的额外要求

当前 aios 提供脚本运行和媒体加工能力，**没有默认安装完整编译工具链**。以下工具应按项目和目标平台另行准备，不能从 `extended` 声明推断它们已存在。

| 任务 | 通常需要的额外命令或环境 |
| --- | --- |
| Rust 开发 | `rustc`、`cargo`，目标平台链接器及项目所需系统库。 |
| C/C++ 或原生扩展 | 平台对应编译器、链接器和 SDK；项目可能要求 `cc`/`c++`、`clang`、`make`、`cmake`、`pkg-config`，Windows 项目可能要求 MSVC/Build Tools。 |
| 前端构建 | `node` 加项目指定的包管理器与依赖，例如 `npm` 或 `pnpm`；Node 存在不代表 pnpm 已安装。 |
| Python 项目 | `python3`、`uv` 或项目指定的依赖管理工具、项目虚拟环境；需要原生扩展时还需编译依赖。 |
| Docker App / 镜像 | `docker` CLI、可连接且有权限使用的 daemon，以及任务所需的构建能力。当前 Agent 容器不自带 Docker。 |
| BuckyOS 系统构建、测试、发布 | 对应源码仓库、目标平台工具链、项目依赖和具体命令要求的身份。当前 aios 不自带 BuckyOS 仓库、`cargo`、`pnpm`、`docker`。 |

具体流程见 [buckyos_app_dev](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev.md) 和 [buckyos_dev_release](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_dev_release.md)。需要其它机器上的能力时，为子 Work/Todo 显式选择相应 runtime；同一 Session 不因缺少一个命令而自动切换执行端。

## 4. cmd_skills 如何使用环境基线

手册使用机器可读字段声明最低环境要求，例如：

```yaml
---
name: video_postprocess
description: 用本地工具检查和合成视频
baseline: extended
platforms: linux, macos, windows
requires: ffmpeg, ffprobe and ImageMagick; use the installed platform's command syntax
---
```

`baseline` 缺省为 `standard`，合法值只有 `standard` 和 `extended`；`platforms` 缺省不限制平台，可写 `linux, macos, windows` 的子集。`requires` 是给人和 Agent 阅读的说明，不被解析为自动安装或逐命令校验规则。

### 4.1 Jarvis cmd_skills 对照表

下表逐项对应 [Jarvis 的 cmd_skills 源码目录](../../src/apps/jarvis_agent/agent/cmd_skills/)：当前共 12 个手册，10 个声明 `standard`，2 个声明 `extended`。它们均未声明 `platforms`，因此当前元数据不限制平台；具体脚本语法、路径和第三方软件仍需适配执行端。

表中的“可选”表示环境基线允许选择该手册，不表示自动注入，也不表示服务、权限和具体工具已全部就绪。“不可选”表示基线不匹配；即使标准环境碰巧装了某个扩展工具，也仍按声明的基线匹配。

| Jarvis cmd_skill | 声明基线 | 标准环境 | 已扩展环境 | 主要命令 / 调用入口 | 实际执行条件与额外依赖 |
| --- | --- | --- | --- | --- | --- |
| [ui_core](../../src/apps/jarvis_agent/agent/cmd_skills/ui_core.md) | `standard` | 可选 | 可选 | `agent-session` 的 Session、Goal、Memory、Notify、skill、task、copy 等操作；`llm_understand_media` | 用于 UI Session，需要有效 Session 上下文；媒体理解需要相应模型。具体状态操作和写入遵循权限。 |
| [xllm](../../src/apps/jarvis_agent/agent/cmd_skills/xllm.md) | `standard` | 可选 | 可选 | `agent_tool xllm` 及其 `status`、`list`、`result` 等子命令 | 需要 Agent 身份和 Zone 的 AICC 模型路由；开启工具后的具体任务另查依赖。独立的 `xllm` 命令名不是要求。 |
| [opendan_agent_dev](../../src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev.md) | `standard` | 可选 | 可选 | `agent-session`；`buckyos pikg`、`buckyos skill`、`buckyos app` | Agent 模板和 skill 编写、打包、管理；模板注册与 Agent 创建需要 owner 操作。源码调试、测试需要另备仓库和 `cargo` 等构建环境，不属于标准保证。 |
| [buckyos_admin](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_admin.md) | `standard` | 可选 | 可选 | `buckyos` 的 `system`、`system-config`、`app`、`log`、`task`、`executor` 等模块 | 需要对应服务和 RBAC；深入源码核验、宿主诊断步骤可能另用 `git`、`sha256sum`、`uv`、`docker`。远端执行还需目标执行器在线并授权。 |
| [share](../../src/apps/jarvis_agent/agent/cmd_skills/share.md) | `standard` | 可选 | 可选 | `buckyos share create/list/get/update/stop/stats` | 文件需位于 Zone 可分享路径且 owner 可读；创建、修改和停止分享需要 `read_write` Agent 权限及可用分享服务。 |
| [aiworkspace_run](../../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run.md) | `standard` | 可选 | 可选 | `buckyos aiws` 的 `run-claim`、`run-files`、`run-program`、`run-tool`、`run-heartbeat`、`run-report`；Deno 程序宿主 | owner 已将 hosted execution 配置给该 Agent，并提供有效 run/claim 上下文。示例使用 `jq`，但它不在标准清单；可改用 Deno 处理 JSON。项目程序的其它依赖单独准备。 |
| [aiworkspace_template_dev](../../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) | `standard` | 可选 | 可选 | `buckyos aiws info/read/query`；Deno + websdk 的 `getAiWorkspaceClient()` | 创建、写入、分享、发布或运行各步骤需要对应 Workspace capability；写操作主要通过 SDK/kRPC 完成，不应假定每个方法都有 CLI 子命令。 |
| [aigc_image](../../src/apps/jarvis_agent/agent/cmd_skills/aigc_image.md) | `standard` | 可选 | 可选 | `gen_image`、`edit_image`、`inpaint_image`、`upscale_image`、`remove_bg`；`llm_understand_media` | 需要所用图像方法的 provider 与资源读取权限；`convert`、`identify`、`compare` 是手册中的可选 ImageMagick 加工/检查步骤，不属于标准要求。 |
| [aigc_audio](../../src/apps/jarvis_agent/agent/cmd_skills/aigc_audio.md) | `standard` | 可选 | 可选 | `text_to_speech`、`speech_to_text`、`gen_music`、`enhance_audio`；按步骤使用 `ffmpeg`、`ffprobe` | 生成需要相应音频 provider 和资源读取权限；转码、拼接、解码及媒体信息检查步骤分别核验 `ffmpeg`、`ffprobe`，编码器按输出格式检查。 |
| [aigc_video](../../src/apps/jarvis_agent/agent/cmd_skills/aigc_video.md) | `standard` | 可选 | 可选 | `gen_video`、`edit_video`、`extend_video`、`upscale_video`；按步骤使用 `ffmpeg`、`ffprobe`、ImageMagick | 生成需要相应视频 provider 和资源读取权限；剪辑、合成、抽帧及本地媒体检查步骤再核验音视频或图像工具，字幕字体、编码器按任务检查。 |
| [buckyos_app_dev](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev.md) | `extended` | 不可选 | 可选 | `buckyos pikg`、`buckyos app`；按项目使用 Node/Deno/Python 及构建命令 | 按 App 类型准备语言、包管理器和项目依赖；Docker App 另需 Docker 环境。安装需要 control-panel 接受的身份；aios 不自带 Docker。 |
| [buckyos_dev_release](../../src/apps/jarvis_agent/agent/cmd_skills/buckyos_dev_release.md) | `extended` | 不可选 | 可选 | `buckyos pikg`、`buckyos app`；系统构建/发布脚本及任务所需的 `uv`、`cargo`、`pnpm`、`docker` 等 | 需要相应源码、目标平台工具链和发布/安装身份。默认 aios 只有部分运行工具，不能直接承担所有系统构建和镜像发布步骤。 |

三个 `aigc_*` 手册还共同使用标准命令 `ai_provider`、`resume_aicc_task` 等进行模型发现和任务跟进；读取已存储的生成结果走 Zone NDM proxy，当前要求 `read_write` Agent。表中涉及的 Deno 可以使用 Tool 自带运行时，host 上不保证 PATH 有 `deno`，见 §2.1 和 §2.4。

这个对照展示的是手册的最低声明基线：**已扩展环境可以选择全部 12 个手册，包含标准环境可选的全部 10 个**；实际执行仍逐项满足最后一列的条件。例如，aios 的媒体工具可以满足音视频加工依赖，但其 `extended` 声明不补齐 Rust、pnpm、Docker 等开发工具。

### 4.2 Session 中的选择与冻结

Session 创建时按自己的 runtime 声明过滤并冻结手册；Workspace 的 runtime 不参与匹配。默认或继承的手册不满足声明的基线或平台条件时不注入，并在提示词中说明原因；显式点名的手册不满足这些条件时拒绝创建，Todo 留在 pending 并记录原因。子 Session 选择其它 runtime 时重新解析；此过程不会逐项验证 `requires` 中的工具。

`extended` 只满足上表的基线条件，不能替代对 `ffmpeg` 等具体依赖的检查。现有手册示例也可能有额外依赖，例如 `aiworkspace_run` 用 `jq` 提取 JSON，而 `jq` 不在标准清单；在纯标准环境上应使用已提供的 Deno 等方式完成该步骤（host 上可能需要使用 Tool 自带 Deno 的绝对路径），或另行准备工具，不能据此扩张标准环境的保证范围。

## 5. 声明、检查与使用

### 5.1 区分三个状态

| 字段 | 表示什么 | 不表示什么 |
| --- | --- | --- |
| `baseline` | 登记者/owner 声明的能力基线，用于 Session 冻结和手册匹配 | 不会安装工具，也不由检测结果自动升降级。 |
| `availability` | runtime 在当前配置和最近检查下是否可用；`check` 会对节点执行器做实际探测 | 不能证明标准工具齐全，更不能证明所有扩展任务可执行。 |
| `baseline_check` | 最近一次标准命令发现检查，记录 `checked_at_ms`、`ok`、`missing`、`error` | 不证明命令行为、版本、动态库、provider 或完整任务已验证，也不核验扩展命令集合。 |

内建节点 runtime 的基线来自 `AgentSettings.runtimes[id].baseline`，可通过 Agent 设置或 `runtimes.settings_set` 修改；自定义 runtime 登记时使用 `xagent runtime register … --baseline standard|extended`。设置 `extended` 之前应实际准备好并说明扩展工具。

检查发现缺少标准命令时，runtime 不满足本文的标准环境要求；当前实现将问题记录为检查结果，**不会自动把 extended 改成 standard，也不会因此改变手册匹配规则**。需要修复执行端的安装或配置。

### 5.2 查看清单和检查目标

以下命令只查看或检查环境：

```sh
# 查看当前程序版本的标准清单，不要求 Agent State 上下文
xagent runtime baseline
xagent runtime baseline --json

# 检查当前执行端 PATH；缺少命令时退出码为 1
xagent runtime baseline --check

# 在 Session 中检查自己所在的执行端；节点上此命令也在本地检查
agent-session runtime baseline --check

# 在 Session 中查看登记，并检查指定的目标 runtime
agent-session runtime list --available
agent-session runtime check host
agent-session runtime get host
```

最后两条的 `host` 应替换为实际登记的 runtime ID。`runtime baseline --check` 检查命令所在机器；`runtime check <id>` 通过登记信息检查目标，两者不能混用。

镜像构建或 Session helper 尚未装配的同机环境可使用 `xagent runtime baseline --check --overlay`：它以 `xagent`、`agent_tool` 的存在替代检查后续由 Session overlay 提供的 `agent-session`、`llm_understand_media`。这只适用于会提供该 overlay 的环境；独立节点需要实际部署完整入口。

### 5.3 核验具体扩展能力

在目标 Session 的 shell 中检查任务需要的工具。以下是音视频环境示例，不代表所有 extended 都有这些工具：

```sh
# Bash / POSIX shell：检查命令发现，再检查程序能启动及其版本
command -v ffmpeg ffprobe convert identify
ffmpeg -version
ffprobe -version
convert -version
```

```powershell
# Windows PowerShell：ImageMagick 7 常用 magick，避免误用系统的 convert.exe
Get-Command ffmpeg, ffprobe, magick -ErrorAction Stop
ffmpeg -version
ffprobe -version
magick -version
```

正式执行前，再用任务所需的最小输入验证编码器、字体、插件或项目依赖。`--version` 成功仍不等于完整任务可执行；FreeCAD 占位入口和 §2.3 的测试入口也说明，只检查命令名存在是不够的。

## 6. 维护与相关文档

标准命令变化时，同步更新 `STANDARD_TOOLS`、命令分派、镜像/host 打包和本文 §2。扩展工具变化时，同步更新实际安装来源、相关 cmd_skill 的依赖说明与本文 §3；不要只改 `baseline` 声明。

- [AgentRuntime 设计 §6.4](<../llm_context/AgentRuntime 设计.md>)：runtime 分层、选择与冻结、执行目录及跨端上下文。
- [Agent State Protocol §10](<protocol/Agent State Protocol.md>)：RuntimeRecord、设置、登记与检查协议。
- [Session Directory Protocol](<protocol/Session Directory Protocol.md>)：冻结配置和 binding。
- [Agent Workspace Protocol](<protocol/Agent Workspace Protocol.md>)：Workspace 位置、访问与文件交接。
- [OpenDAN Agent 开发指南 §2.3–2.4](../sdk/OpenDAN_Agent_Dev_Guide.md)：工具源码、cmd_skills 的编写与分发。
- [Session Runtime 改造记录](../../notepads/opendan-primary-runtime-tools-todo.md)：设计裁决、实施差异与历史验证记录；当前环境工具范围以源码和本文为准。
