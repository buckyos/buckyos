# BuckyOS App + OpenDAN 的容器与卷升级技术需求文档



## 1. 背景

BuckyOS 早期的容器模型强调“镜像无状态、数据在宿主机”：

1. Docker 镜像只承载二进制与运行环境。
2. 所有持久化状态都通过挂载（mount）进入容器。
3. Docker 容器本身不保存业务状态，容器只是特殊的进程启动方式。

这一模型在数据安全、权限控制、备份恢复上非常清晰：宿主机文件系统是数据事实来源，容器不是数据边界。

但随着 ScriptHost 类应用与 OpenDAN Agent 类应用的发展，该模型出现了明显分叉：

- **ScriptHost App** 需要在容器中运行 TypeScript（Deno）与 Python（uv）脚本，并隔离各自的 npm/pip/uv 依赖。
- **OpenDAN Agent App** 天然存在提示词、规则、脚本的本地自演化、自修改与上游版本合并需求。

继续坚持“容器绝对无状态 + 全部状态只在宿主机明文目录中”的旧模型，会导致跨平台实现复杂度、运行不稳定性、权限语义分裂与维护成本持续上升。因此需要将容器与卷模型升级为统一、显式、可诊断、可迁移的新架构。

## 2. 当前问题

### 2.1 ScriptHost 的依赖安装成本与不稳定性

ScriptHost 的脚本主体可以放在宿主机上，但其运行依赖并不适合在每次容器重启时重新安装：

- 每次启动重新安装依赖会显著增加启动时间。
- 复杂 pip/npm/uv 依赖在“每次重建都重新安装”的模式下会引入随机失败。
- 同一个应用在首次安装成功后，本应进入相对稳定状态；但无状态容器会让这种稳定性无法保留。

当前“为每个 App 单独挂一个 dependency volume”的方案，只解决了一部分问题，但语义仍偏向“半临时缓存”，没有形成统一、清晰的状态边界。

### 2.2 只读脚本挂载无法真正阻止运行期自修改

对于脚本类应用，过去将宿主机脚本目录以只读方式挂入容器，目的是限制应用在运行时修改自身代码。但实践中：

- 应用可以把真正逻辑转移到可写的依赖区。
- 应用可以通过依赖升级、下载 loader、动态拉取代码等方式绕过“原始脚本只读”的限制。
- 在供应链安全本身未被根本解决的前提下，“只读挂载原始脚本”并不能构成真正有效的安全边界。

因此，系统需要承认“运行期可变代码”这一事实，并把它限制在明确的、可重置的私有边界内，而不是依赖一个容易被绕过的伪不可变模型。

### 2.3 OpenDAN 现有 COW 合并方案过于复杂且跨平台脆弱

OpenDAN Agent 一开始就存在自改提示词、角色描述、流程规则与任务处理脚本的需求。现有方案依赖文件系统级 COW/覆盖语义来合并：

- 上游开发者发布的版本
- Agent 本地自演化后的版本

这一方案在 Linux 上尚可实现，但在 Windows/macOS + Docker Desktop + 不同文件系统/不同 Docker 引擎版本下，细节复杂、兼容性差、问题多发，已经成为持续演进的阻力。

### 2.4 Agent 与普通 App 的安全模型正在分叉

当前一条分支强调 ScriptHost 运行期尽量不可变，另一条分支则允许 OpenDAN 进行自演化。这导致：

- 两类 App 的容器管理逻辑逐渐分裂；
- 安全边界、权限控制、升级机制不一致；
- “Agent 只是特殊 App”这一根本原则开始被削弱。

长期看，这会让安全模型失控，尤其不适合生产环境。

### 2.5 Desktop 与标准独立安装环境需要统一语义

未来 Desktop 环境与标准独立安装环境都需要支持同样的 App/Agent 生命周期、卷语义与迁移路径。如果继续依赖不同平台文件系统的特殊行为，后续迁移与兼容成本会持续放大。

### 2.6 Agent 能力扩展与镜像能力分层的缺失

随着意图引擎的分层设计落地，Agent 的工作被划分为两层：

- **工作层（Worker）**：Agent 只能组合既有 CLI 工具或使用 Deno 做编排，不得在运行期安装新依赖。这一层要求运行环境确定、无编译工具链、失败面小、启动快。
- **工匠层（Crafter）**：由专门的 Agent 负责打造新 CLI 工具（通常涉及 Python 及其原生依赖编译），产物以工具包形式交付给工作层使用。

当前系统只提供一种 App/Agent 镜像，无法表达这种分层：

- 若统一用“完整带编译工具链”的镜像，则工作层 Agent 运行环境臃肿、启动慢、攻击面大，且无法对“是否允许扩展工具集”做权限隔离。
- 若统一用“精简运行时”的镜像，则工匠 Agent 无法完成编译与依赖安装任务，打造新兵器的能力无处落地。
- Python 依赖链的复杂性（需要 dev headers、系统库、多版本 Python、跨语言工具链）若污染到工作层镜像，会严重放大镜像体积与失败率，抵消工作层“确定性执行”的设计价值。

因此，系统需要在镜像层引入一种明确的能力分层，使得“运行既有能力”和“构造新能力”这两件事运行在不同的镜像与权限边界中，而不是混在同一个镜像里。该分层还应当作为用户侧的可控开关：默认只启用工作层能力，工匠层能力由用户显式开启。

## 3. 设计目标

本次升级的核心目标如下：

1. **统一 BuckyOS App 与 OpenDAN Agent 的容器管理模型。**
2. **引入“应用实例卷（Instance Volume）”作为应用私有可变状态的标准承载体。**
3. **将“应用私有状态”与“外部业务数据/共享数据”明确分离。**
4. **让容器仍然保持可重建，但不再要求应用的全部可变内容都放在宿主机明文目录中。**
5. **用文件级同步/合并替代文件系统级 COW，实现跨平台一致性。**
6. **允许运行期代码与依赖发生变化，但必须局限在可重置、可诊断、可回退的实例边界内。**
7. **坚持默认最小权限原则，权限扩大必须通过显式挂载或显式工具能力完成。**
8. **为安全模式、故障诊断、卷重建、迁移升级提供统一基础。**
9. **在镜像层引入工作镜像（Worker Image）与工匠镜像（Crafter Image）的明确分层，使“运行既有能力”与“构造新能力”位于不同的镜像与权限边界，并默认关闭工匠能力。**
10. **引入ExtTool Volume,解决新工具的安装与分发的问题**

## 4. 非目标

本次升级不以以下事项为目标：

1. 不让 App/Agent 默认获得宿主机全盘访问权。
2. 不把“运行期代码绝对不可变”作为硬性安全承诺。
3. 不在第一阶段解决所有 npm/pip/uv 等供应链安全问题。
4. 不在第一阶段实现跨 App 的智能依赖去重与全局共享缓存最优解。
5. 不允许以直接共享私有实例卷的方式实现 App 间协作。
6. 不要求工匠镜像默认启用；打造新兵器始终是一项需要用户显式授权的能力。
7. 不要求工匠镜像本身作为工具交付物；工匠镜像只是生产工具的场所，交付给工作层的是独立的工具包。
8. 不再第一个版本实现工匠镜像

## 5. 核心概念与术语

### 5.1 镜像（Image）

应用镜像只承载基础运行环境、启动器与必要二进制。镜像本身应尽量保持无状态与可重复构建。

根据承担的角色，系统区分两类镜像：

- **工作镜像（Worker Image,paios/aios）**：对应意图引擎的工作层。只包含运行时、脚本语言（Deno、受限的 Python）与一组预置的 CLI 工具，不包含编译工具链、不包含完整的 dev headers、不允许在容器内通过包管理器安装新的系统级依赖。工作镜像是所有 App/Agent 默认运行环境。
- **工匠镜像（Crafter Image）**：对应意图引擎的工匠层。在工作镜像的能力基础上，额外叠加完整的编译工具链、多版本 Python、Rust/Go 等语言工具链、系统 dev headers、测试框架，以及“模拟工作镜像环境”的验证能力，用于打造新的 CLI 工具或工具包。

工匠镜像在文件系统继承关系上是工作镜像的超集，但两者在运行角色、权限边界、默认可用性、资源预期上是不同的，不得被视为同一个镜像。

### 5.2 工具包（Tool Package）

工匠层产出的交付物。工具包不是镜像，而是一个独立的、可被工作镜像加载的能力单元，至少包含：

- 可执行入口与 bundle 的非系统库依赖
- 能力描述（schema）
- 构建元数据（版本、哈希、作者/工匠 Agent、构建环境）
- 所需的工作镜像侧系统依赖声明
- 验收测试用例

工具包通过统一的ExtTool Volume进入工作镜像的能力视图，成为工作层 Agent 可组合的 CLI 工具。工作层 Agent 只看到工具包暴露的 CLI 接口，不关心其内部实现语言与构建过程。

### 5.3 运行时容器（Runtime Container）

容器是由镜像创建出的运行实例。容器本身可以是短生命周期、可删除、可重建的；持久化语义不再依赖容器文件系统本身，而依赖外部卷与显式挂载。

### 5.4 上游发行内容（Release Source）

由 BuckyOS 发布体系或应用开发者发布的、具有明确版本与哈希语义的原始代码/脚本/规则内容。该内容在系统视角下是只读、可校验、可替换的。

### 5.5 应用实例卷（Instance Volume）

每个应用实例拥有的私有可写卷。用于承载：

- 运行时重要缓存
- 依赖安装结果
- 本地可变代码/工作副本
- OpenDAN 的本地演化提示词/规则/脚本
- 其他可重建但不适合每次启动都重新生成的私有运行状态

实例卷不是纯 cache，也不是跨 App 共享存储；它表示“应用实例自己的可变私有层”。

### 5.6 外部数据挂载（External Mount）

由系统显式挂入容器的宿主机目录或系统文件系统目录。用于承载：

- 用户文档
- 业务数据
- 需要长期保留的数据
- 需要被多个 App 共享的数据

### 5.7 共享数据目录（Shared Data）

若多个 App 需要共享数据，必须通过系统认可的外部挂载路径实现，而不是直接共享某个 App 的实例卷。

### 5.8 安全模式（Safe Mode）

应用启动时不使用原有实例卷，而改用新的空卷或临时卷启动的模式。用于故障隔离、诊断、回滚验证。

## 6. 目标架构

### 6.1 统一分层模型

升级后的统一模型如下：

```text
[Immutable Image / Release Source]
            │
            ├─ 只读上游发行内容（可来自镜像或只读挂载）
            │
[Private Instance Volume]
            ├─ 依赖
            ├─ 运行时缓存
            ├─ 本地可变代码/工作副本
            └─ Agent 自演化内容

[External Tool Volume]
            │
            ├─ 后续添加的新Agent工具
[Explicit External Mounts]
            ├─ 用户文档
            ├─ 业务数据
            └─ App 间共享数据
            │
[Ephemeral Temp]
            └─ 临时文件、临时缓存、会话级中间结果
```

### 6.2 新的状态定义

从系统角度，应用状态被划分为两大类：

1. **私有运行状态**  
   必须落在实例卷中，只属于当前 App 实例。

2. **外部业务/共享状态**  
   必须通过显式挂载进入容器，不能隐式放在实例卷中作为长期业务事实来源。

换言之：

- 实例卷表达“App 自己的私有、可变、可重置层”；
- 外部挂载表达“系统明确授权给 App 访问的外部数据边界”。

### 6.3 镜像分层模型

镜像层面引入工作/工匠两级分层。文件系统上使用继承关系表达，运行角色上保持独立：

```text
[Base Image]
    └─ ca-certs / Deno / uv / 最小系统库（locale 用 glibc 自带的 C.UTF-8；
       字体随 ext-core 的 LibreOffice，不装进镜像）
            │
            ▼
[Worker Image]（默认运行镜像，paios/aios）
    └─ 预置 CLI 工具集：§6.4 表 B 的清单（ffmpeg、ImageMagick、
       poppler、tesseract、LibreOffice headless、字体、git、jq …）
    └─ Python 3.12 + uv、Node LTS、Deno：胶水与 ScriptHost 依赖，
       不在镜像内装系统包
            │（仅当用户显式启用工匠能力时继续向上）
            ▼
[Crafter Image]（可选，默认关闭）
    └─ 完整编译工具链（gcc/clang/cmake/pkg-config）
    └─ 多版本 Python + 完整 dev headers
    └─ Rust / Go / Node 等额外语言工具链
    └─ 工作镜像环境模拟器（用于验证工具包在 Worker 中可运行）
            │
            ▼
[External Tool Volume]（独立交付物，非镜像）
    └─ 挂到 /opt/buckyos/tools；今天由 publish/exttool/ 的 seed 镜像
       按镜像 id 播种一次（pandoc 已是 ext-core 的一部分，§6.4 表 B；
       可选 FreeCAD、uv/deno 预热缓存见表 C）
    └─ 目标：扩展包体系——系统管理员按 Zone 勾选扩展包（pikg，
       app_type = ext_tools，包间有依赖），node_daemon 在每个节点
       reconcile 到节点级卷 buckyos-tools；ext-core 是第一个包
       （notepads/opendan-exttools-cross-platform-todo.md §3.9）
```

要点：

1. 工作镜像是所有 App/Agent 的默认运行环境，必须保持精简、启动快、失败面小。
2. 工匠镜像是工作镜像的文件系统超集，但**不作为默认运行环境**；它只在用户显式开启“打造新兵器”能力时，为工匠 Agent 启动。
3. 工匠镜像的产物是独立的工具包，不是“工匠镜像本身替代工作镜像”。工具包加载回工作镜像后，工作层 Agent 才能使用新能力。
4. 工匠镜像必须具备“模拟工作镜像环境”的能力，以便在交付前验证产物能在干净的工作镜像里运行，避免构建环境与运行环境之间的隐式依赖泄漏。
5. 体积大、但只有部分任务用到的 CLI（目前是 pandoc）以自包含工具包的形式随 External Tool Volume 发布（`/opt/buckyos/tools/bin/` 在 PATH 上），不进工作镜像。External Tool Volume 不可用时 Worker 照常启动，只是缺这些工具（node_daemon 记 error 日志）。ExtTools TODO T2 之后整组 `ext-core` 命令都这样交付：容器节点的 `buckyos-tools` 卷与原生的 `$BUCKYOS_ROOT/tools/` 用同一份 payload 字节（§6.4 表 B）。
6. Agent 看到的工具分三层（owner 2026-11-05 定）：**核心**（Agent 标准命令及其运行依赖，随系统升级、不能独立升级）、**扩展**（zone-owner 装的扩展包，可独立升级，装在哪台机器上对该机器所有 Agent 生效）、**Agent 自装**（在自己的实例卷里，删掉不伤核心）。工作镜像预置的 CLI 工具集在扩展包体系落地后就是 `ext-core` 的内容；T2 之后工作镜像不再 apt 装这组命令，容器也从 `ext-core` payload 取（§6.4 表 B）。`ext-core` 1.2.0 起 LibreOffice headless 与字体也在其中：`build_aios` 的 `RUNTIME_PACKAGES` 只剩 `bash`、`ca-certificates`、`rsync`，不装 `locales`（`LANG=C.UTF-8`）。

### 6.4 官方发布的基线与扩展工具清单（运维事实来源）

**本节是“官方发布的 Agent 可用命令”的唯一事实来源，也是每个官方扩展包内容的唯一事实来源。** 规则：

- 每条命令必须写明**支持的场景**（为什么要装、哪些 skill / App 用）。没有场景的命令不进官方清单；去掉一条命令要先列出受影响的场景并更新本表。
- **机制优先于清单（owner 2026-11-06）**：扩展包体系与 `ext-core` 的发行、钉版、种子流程先建好（ExtTools TODO §3.10）；首版 `ext-core` 的清单可以宽，实现需要的工具都可以先放进 `ext-core`，但**每一条都必须先写进表 B**。后续调整清单走 `ext-core` 升版与 pikg 更新，不走代码特例。
- 代码与构建脚本必须与本表一致，由测试守：`baseline::STANDARD_TOOLS` ↔ `src/tools/buckyos-agent/commands/*.ts`（已有）；`baseline::EXTENDED_TOOLS` ↔ `ext-core` 的 `ext-manifest.json`（已有）；表 B ↔ `ext-manifest.json` 的 `provides`、表 B 的来源列 ↔ `publish/exttool/ext-core/build.sh` 的锁定清单（ExtTools TODO T2b 加脚本检查）。改任一处先改本表。
- 新场景需要的新命令走扩展包（ExtTools TODO §3.9），不往核心层加，也不单独往工作镜像加；工作镜像里的 Agent 可用命令只随 `ext-core` 与表 C 的包变。
- 版本锁在各自构建脚本顶部：`build_aios` 的 `NODE_VERSION`、`DENO_VERSION`（核心层自己钉死的 Deno）；`publish/exttool/ext-core/build.sh` 顶部的官方归档版本，SHA256 在同目录 `sources.sha256`；conda-forge 那组由同目录 `conda-env.yml` 声明、`conda-lock.yml` 锁定。
- 非官方来源（Agent 自装、用户在 host 上自备、第三方扩展包）不在本表；它们由 runtime 的 `tools_check` 按实际探测记录（`provided_by: agent | system`）。

**表 A：核心层（standard，`baseline::STANDARD_TOOLS`，31 条）** 随系统发布，三种形态（容器、Desktop Lite、node-daemon host）都必须有；依赖的 Deno、TS 源码与 SDK 由 launcher 钉在 `$BUCKYOS_ROOT` 下的绝对路径，不走 PATH。

| 命令 | 是什么 | 支持的场景（为什么要装） |
| --- | --- | --- |
| `agent_tool` | Session 工具的实现载体（shell / read / write / xllm / 媒体理解等） | 所有 Session；`llm_understand_media` 是它的入口别名 |
| `xagent`、`agent-session`（别名） | Session 对宿主 Agent State 的控制面：create-worksession、todo、copy、run、skill、goal、notify、task、recall、runtime… | `ui_core` 与 work / plan / goal 的全部行为；只在私有 runtime 的 Session 里使用 |
| `buckyos` | BuckyOS CLI：system / system-config / app / log / task / executor / share / aiws / pikg / skill | `buckyos_admin`、`share`、`aiworkspace_run`、`aiworkspace_template_dev`、`opendan_agent_dev`、`buckyos_app_dev`、`buckyos_dev_release` |
| `aicc-tool` | AICC 任务的底层 CLI | 媒体命令与 `xllm` 的共同依赖 |
| `ai_provider`、`ai_provider_test`、`ai_quota` | 模型 / provider 发现、连通测试、额度 | `aigc_*` 选 provider；`xllm`；“Zone 有没有视频模型”这类判断的证据来源 |
| `ask_user` | 经 notify 向用户提问 | work / goal 需要人答时 |
| `import_office`、`import_table` | 把 Office 文档 / 表格导入为对象 | 文档与数据导入、知识整理 |
| `materialize_resource` | 把对象 / URL 物化成本地文件 | 任何要把 `obj://`、附件落成文件再加工的步骤 |
| `resume_aicc_task` | 续接异步 AICC 任务 | `aigc_*` 的长任务跟进 |
| `llm_understand_media` | 看图、读文档（PDF…）、听音视频，结果是文本 | `ui_core` 的媒体理解；`aigc_image` 的结果检查 |
| `caption_image`、`detect_image`、`segment_image`、`ocr_image` | 图像理解：描述、检测、分割、OCR（走 Zone 模型） | 图像类任务的理解步骤；`ocr_image` 是 OCR 的主路径 |
| `gen_image`、`edit_image`、`inpaint_image`、`upscale_image`、`remove_bg` | 图像生成与编辑（走 Zone 模型） | `aigc_image` |
| `gen_video`、`edit_video`、`extend_video`、`upscale_video` | 视频生成与编辑（走 Zone 模型） | `aigc_video`；本地剪辑 / 合成另需表 B 的 ffmpeg |
| `gen_music`、`text_to_speech`、`speech_to_text`、`enhance_audio` | 音频生成、TTS、ASR、增强（走 Zone 模型） | `aigc_audio`；本地转码 / 拼接另需表 B 的 ffmpeg |

**表 B：`ext-core` 扩展包（`extended` 基线，`baseline::EXTENDED_TOOLS` ↔ `publish/exttool/ext-core/ext-manifest.json`，24 条命令）**

`ext-core` 是扩展包目录里的第一个包，五个平台（`linux-x86_64`、`linux-aarch64`、`darwin-arm64`、`darwin-x86_64`、`windows-x86_64`）各一个自包含 payload；装到 `$BUCKYOS_ROOT/tools/`，三种形态一套（容器节点是 `buckyos-tools` 卷）。发行、钉版与种子见 ExtTools TODO §3.10。工作镜像不再 apt 装这组命令，容器与原生一样从 `ext-core` payload 得到它们。

来源列的含义（owner 2026-11-06 裁决）：**官方** = 上游官方发布的归档，`build.sh` 逐工具下载并固定版本与 SHA256，落 `store/ext-core/<tool>/`；**conda-forge** = 没有官方二进制（或某个平台没有）的工具，统一装进一个 conda-forge 前缀 `store/ext-core/conda/`，由 `conda-lock.yml` 锁版本与来源；ELF / Mach-O 的 rpath 本来就相对于自身，数据目录由 launcher 用环境变量指过去（T0 实测，不需要 conda-pack 的安装后重写，见 ExtTools TODO §4.2）。`bin/` 里的 launcher 只指向 `store/` 内的文件，**不回退到 PATH**（缺就退 127），这样 `extended` 的检查结果不受机器环境影响。

| 命令 | 工具 / 版本锁 | linux x86_64 / aarch64 | macOS arm64 / x86_64 | windows x86_64 | 支持的场景（为什么要装） |
| --- | --- | --- | --- | --- | --- |
| `ffmpeg`、`ffprobe` | ffmpeg，`conda-lock.yml`（conda-forge 的构建没有 libzimg，没有 `zscale` 滤镜） | conda-forge | conda-forge | conda-forge | `aigc_audio` / `aigc_video` 的转码、剪辑、拼接、抽帧、媒体信息检查；workflow 媒体工具基线 |
| `magick`、`convert`、`identify`、`compare` | ImageMagick 7，`conda-lock.yml`。`convert / identify / compare` 是兼容入口：IM7 自带旧名时直接用，否则 launcher 映射到 `magick <子命令>` | conda-forge | conda-forge | conda-forge | `aigc_image` / `aigc_video` 的本地图像检查与加工。随本表统一到 IM7（原 ExtTools TODO T0 的 IM6 / IM7 待决项按此关闭） |
| `pdftotext`、`pdftoppm`、`pdfinfo` | poppler ≥ 22.0，`conda-lock.yml` | conda-forge | conda-forge | conda-forge | 媒体工具基线（[doc/workflow/media tool baseline.md](../workflow/media%20tool%20baseline.md)）：workflow `media.pdf` / `media.document` 的 PDF 文本 / 页图提取；`import_office` 后加工 |
| `tesseract`（含常用语言数据） | tesseract，`conda-lock.yml`（conda-forge 带全部 126 种语言，tessdata_fast；构建时保留 28 种加 `osd`：系统界面语言 `chi_sim`、`chi_tra`、`eng`、`spa`、`fra`、`deu`、`kor`、`jpn`、`rus`，竖排 `chi_sim_vert`、`chi_tra_vert`、`jpn_vert`、`kor_vert`，其他常用 `por`、`ita`、`nld`、`pol`、`tur`、`ukr`、`ara`、`fas`、`heb`、`hin`、`ben`、`tha`、`vie`、`ind`、`msa`；保留清单在 `build.sh`，`smoke/tesseract.*` 逐个检查） | conda-forge | conda-forge | conda-forge | workflow 媒体基线：`media.ocr_pages` 为文字层缺失或乱码的 PDF 页重新识别（[media tool baseline.md](../workflow/media%20tool%20baseline.md)）；Agent 本地 OCR |
| `pandoc` | 3.12.1，`build.sh` | 官方 | 官方 | 官方 | 文档格式转换（docx / md / html / pdf 互转）：`import_office` 的后加工、文档类产出 |
| `soffice` | LibreOffice 官方发布版 26.8.2，`build.sh`。只取无界面转换要用的子集：core、ure、writer、calc、impress、draw、math、graphicfilter、xsltfilter 和它自带的字体；pyuno、内置 Python、Java 类、帮助、图库、图标主题、扩展、词典、数据库驱动、GUI 媒体后端都不要。另带 Noto Sans / Serif CJK 的常规体和粗体（OTC，`Sans2.004` / `Serif2.003`），五个平台同一套，放在 LibreOffice 自己的字体目录。版本在 `build.sh` 顶部，SHA256 在 `sources.sha256` | 官方 deb 包解包（`build.sh` 自己读 `ar`，不需要 dpkg）。官方构建从系统取的库（cairo、cups、dbus、fontconfig、freetype、glib、NSS / NSPR、X11 一族、Kerberos、avahi、libstdc++）全部来自同一个 conda-forge 前缀，其中只有 `avahi` 是为它新加的（`conda-env.yml`）；launcher 把 `LD_LIBRARY_PATH` 设成 LibreOffice 自己的 `program/` 加前缀的 `lib/`，`FONTCONFIG_FILE` 指向前缀的配置（不读系统字体目录；没写语言的中日韩文字回退到简体字形），环境里没有 UTF-8 locale 时设 `LC_ALL=C.UTF-8`。**要求 glibc ≥ 2.34**（`ext-core` 其余命令是 2.28）：Debian 11、RHEL 8 这一代上 `smoke/soffice.sh` 不过，整个包装不上 | 官方 dmg 里的 LibreOffice.app（`hdiutil`，只能在 Mac 上构建），arm64 和 x86_64 分开；launcher 指向 `Contents/MacOS/soffice` | 官方 msi 解包：在 Windows 上用 `msiexec /a` 做管理安装，在其它系统上用 `msiextract`（msitools）；msi 里装进系统字体目录的字体改放到 `share\fonts\truetype`；launcher 指向 `program\soffice.com` | Preview 管线：把 Office / ODF / WPS 文档转成 PDF，把 xls / ods 等旧表格转成 xlsx。workflow 媒体基线 `media.to_pdf`。`import_office` 的原样模式和不可解析对象的栅格化。字体集算在转换指纹里（`env/fonts`），所以各平台带同一套 |
| `python3` | python-build-standalone `install_only` 3.12.x，`build.sh` | 官方（astral） | 官方（astral） | 官方（astral） | ScriptHost Python App 的依赖安装；Agent 临时脚本。项目解释器由 uv 管，这份只是保证 `python3` 存在 |
| `uv` | `build.sh` | 官方 | 官方 | 官方 | Python 项目与解释器管理；`UV_CACHE_DIR` 等指到 Agent 缓存（ExtTools TODO §3.6） |
| `node`、`npm`、`npx` | Node LTS，`build.sh`（与 `build_aios` 的 `NODE_VERSION` 同一个数） | 官方 | 官方 | 官方 | ScriptHost / aiws 项目构建；`buckyos_app_dev` 的 web 包 |
| `deno` | `build.sh`（与 `build_aios` 的 `DENO_VERSION` 同一个数） | 官方 | 官方 | 官方 | ScriptHost TS、aiworkspace 程序宿主、`aiworkspace_run` 的 JSON 处理。核心命令用的是自己钉死的那份 Deno，不是它 |
| `git` | `conda-lock.yml`；Windows 用 MinGit | conda-forge | conda-forge（**不回退** `/usr/bin/git`：它是 Xcode CLT 垫片，会弹安装对话框） | 官方 MinGit | `buckyos_app_dev`、`opendan_agent_dev`、`buckyos_dev_release` 的源码操作 |
| `curl` | `conda-lock.yml` | conda-forge | conda-forge | conda-forge | 下载；Agent 脚本的 HTTP 调用 |
| `jq` | `build.sh` | 官方 | 官方 | 官方 | JSON 提取（`aiworkspace_run` 示例、skill 脚本） |
| `tree` | `conda-lock.yml`；win-64 没有包，也没有官方二进制，用随包的 `windows/tree.py`（`-a -d -L`，同样的输出格式） | conda-forge | conda-forge | 随包脚本（跑在 ext-core 的 python 上） | 目录查看 |
| `unzip` | `conda-lock.yml`；win-64 没有包，用 busybox-w32 官方单文件（`build.sh`） | conda-forge | conda-forge | 官方（busybox-w32） | 归档解压 |
| `xz` | `conda-lock.yml` | conda-forge | conda-forge | conda-forge | 归档压缩 / 解压 |

`git`、`curl`、`tree`、`unzip`、`xz` 这组辅助命令在五个平台都随 `ext-core` 带，launcher 不依赖系统同名命令（owner 2026-11-06 确认；macOS 的系统 `git` 是 Xcode CLT 垫片，会弹安装对话框）。锁定清单与构建脚本都在 `publish/exttool/ext-core/`：`build.sh` 管官方归档，`conda-lock.yml` 管 conda-forge 那组。

`bash`、`rsync` 是 Linux 特有，仍由 `build_aios` 的 `RUNTIME_PACKAGES` 进工作镜像，不在 `ext-core`，也不要求 macOS / Windows 提供。

**owner 2026-11-10 定：工作镜像里的工具都来自 `ext-core`。** LibreOffice 进了工作镜像，就属于核心扩展工具：`soffice` 命令一定可用，各环境的区别只是调用方式不同（容器从工具卷取，宿主从 `$BUCKYOS_ROOT/tools` 取）。所以原来规划的 `ext-office` 并入 `ext-core`，也就是上表的 `soffice`（`ext-core` 1.2.0）。`build_aios` 里原来那组 apt 包（`libreoffice-{core,writer,impress,calc}-nogui`、四个字体包、`locales`）已删除。

**表 C：其它官方扩展包与非包内容**

每个官方扩展包都是一个 pikg（`app_type = ext_tools`），各有自己的 `ext-manifest.json` 与本表的一张清单；新包先在这里登记场景与命令，再建工程。

| 包 / 内容 | 命令 | 状态 | 来源 / 版本 | 支持的场景（为什么要装） |
| --- | --- | --- | --- | --- |
| `ext-cad`（规划） | `freecadcmd` | 规划中，默认不装。今天是 exttool 镜像的可选项：`FREECAD_APPIMAGE_URL_*` 为空时没有这个命令 | FreeCAD 官方 AppImage（Linux x86_64；aarch64 核）、dmg、7z | CAD 脚本化 |
| 工具卷的 `uv-cache`、`deno-cache` | 不是命令 | 随 `publish/exttool` seed 镜像预热，卷新建时复制一次；格式不允许包携带缓存（X3 定：留在 seed 镜像） | 随镜像 | ScriptHost 首启离线装依赖 |

**不在官方清单里、手册可能提到的**：`cargo`、`pnpm`、`docker`、`sha256sum`、编译工具链（`buckyos_dev_release`、`buckyos_app_dev` 的 Docker App）——属于工匠层或 Agent 自装，`extended` 不保证。

## 7. 核心需求

### 7.1 容器与卷的一致性需求

**R-1** 系统必须支持“容器可重建、实例卷持久化”的运行模型。  
**R-2** 应用重启、容器重建、镜像更新不应默认导致实例卷丢失。  
**R-3** 删除实例卷只应影响该 App 的私有运行状态，不得连带删除外部挂载中的用户数据与共享数据。  
**R-4** ScriptHost App 与 OpenDAN Agent App 必须接入统一的实例卷模型。  
**R-5** 纯二进制、完全无私有状态的 App 可以在后续通过声明方式选择不使用实例卷，但系统默认应支持实例卷能力。  

### 7.2 数据放置规则

#### 7.2.1 必须放入实例卷的内容

**R-6** 以下内容必须优先放入实例卷：

- Deno/uv/npm/pip 等运行时依赖与下载缓存
- 安装后需要复用的重要运行时缓存
- 应用在运行期修改后的本地代码/脚本/规则
- OpenDAN 的自演化提示词、流程规则、任务模板、局部工作副本
- 其他“每次重建都重新生成成本高，且只属于该 App”的内容

#### 7.2.2 不应放入实例卷的内容

**R-7** 以下内容不应只保存在实例卷中：

- 用户文档
- 业务核心数据
- 需要跨 App 共享的数据
- 用户预期长期保留且不应因“重置 App”而丢失的数据

**R-8** 若应用将某些运行期生成内容视为重要业务成果，则必须显式导出到外部数据目录，并在应用文档中说明其持久化策略；系统不保证实例卷重置后仍保留这些内容。

### 7.3 私有代码层与上游发行层的关系

**R-9** 系统必须把“上游发行内容”与“本地可变工作副本”分离。  
**R-10** 上游发行内容应保持只读与可校验语义。  
**R-11** 应用在运行期的自修改、自升级、自演化应落在实例卷中的私有可变层。  
**R-12** 不得再把“宿主机挂入的原始脚本目录只读”作为主要安全边界；真正边界应是“是否显式挂载了外部数据，以及挂载了什么权限”。  

### 7.4 升级、同步与合并需求

**R-13** 系统必须用文件级同步/合并机制替代对宿主机文件系统 COW 语义的依赖。通常是一个确定的容器启动脚本  
**R-14** OpenDAN 的本地演化内容与上游发版内容的处理，必须能够在 Windows、macOS、Linux 上保持一致语义。  
**R-15** 最低可行的升级策略应满足：

- 本地不存在的文件：从上游复制到实例卷
- 本地存在且未被本地修改的文件：可被新的上游版本覆盖/更新
- 本地已被本地修改的文件：默认保留本地版本，不被静默覆盖

**R-16** 系统应该记录必要的版本/哈希元数据，以支撑“是否发生本地修改”的判断。  
**R-17** 文件冲突处理策略至少要做到“可预测、可记录、可诊断”，不允许依赖平台相关的隐式文件系统行为。  

### 7.5 实例卷删除、重建与安全模式

**R-18** 删除实例卷必须被视为一种可预期的“重置 App 私有状态”操作，而不是单纯“清缓存”。  
**R-19** 删除实例卷后，应用应能回到接近原始发行状态，并允许重新构建依赖与本地工作副本。  
**R-20** 对于 OpenDAN 或脚本类 App，删除实例卷后丢失本地演化代码/自升级代码应被明确定义为系统特性，而不是异常。  
**R-21** 系统应该支持安全模式启动：启动时不加载原有实例卷，而是使用新的空卷或临时卷运行。  
**R-22** 安全模式应便于诊断“旧卷损坏/本地演化出错”与“应用本体损坏”之间的差异。  

### 7.6 App 间共享与边界控制

**R-23** 一个 App 的实例卷默认不得直接挂载给另一个 App 使用。  
**R-24** 如果 App1 需要向 App2 共享数据，必须通过系统认可的共享目录或外部挂载路径中转。  
**R-25** 系统应在概念与实现上区分“私有卷”与“共享目录”，避免语义混淆。  

### 7.7 权限与安全模型需求

**R-26** 默认权限模型必须继续坚持最小权限原则。  
**R-27** App/Agent 能访问哪些宿主机目录，必须由显式挂载决定，而不是由它是否拥有实例卷决定。  
**R-28** 默认安装配置下，Agent 不应自动拥有宿主机全盘访问权。  
**R-29** 若需要更大权限（例如访问更多宿主机目录、执行宿主机原生命令），必须通过显式挂载策略或显式桥接工具能力实现。  
**R-30** Agent 的权限扩展方式不得破坏统一 App 安全模型；“Agent 是特殊 App，而不是例外中的例外”必须成立。  

### 7.8 工匠镜像与工具包需求

工匠镜像不在本版本讨论。工具包的格式与分发已在扩展包体系里定下（pikg `app_type = ext_tools`、`ext-manifest.json` 的 `provides / requires / conflicts`、Zone 级期望状态、node_daemon reconciler），见 `notepads/opendan-exttools-cross-platform-todo.md` §3.9；工匠层落地时产出的就是这种包。

### 7.9 跨平台兼容需求

**R-45** 新架构必须在 Windows + Docker Desktop、macOS + Docker Desktop、Linux 原生/标准环境中提供一致的逻辑语义。  
**R-46** 新架构不得依赖宿主机文件系统的特定 COW/overlay/hardlink 语义作为核心正确性前提。  
**R-47** Desktop 与标准独立安装环境应共享同一套卷分类、重置语义、升级语义与挂载策略定义。  

### 7.10 配额、空间治理与可观测性

**R-48** 系统需要为实例卷提供基本的容量管理能力，至少包括：使用量可见、可清理、可重置。  
**R-49** 应用声明层面应支持定义或建议实例卷配额。  
**R-50** 卷压缩、卷去重、跨 App 共享缓存等高级能力可作为后续阶段，但本期至少要预留概念空间，不能阻塞后续扩展。  
**R-51** 工匠镜像的磁盘占用、构建历史、失败构建的临时产物清理，应纳入可观测与可治理范围，并与工作镜像的观测指标分开呈现。  

### 7.11 用户与开发者可感知语义

**R-52** 应用开发者必须能够清楚知道：不同目录放什么内容，会决定数据在“重启、升级、重置、安全模式”下的行为。  
**R-53** 系统必须提供明确文档，说明哪些目录属于：
- 上游发行内容
- 实例卷私有层
- 外部业务数据层
- 临时层

**R-54** 对终端用户而言，“重置 App”与“删除业务数据”必须是两个不同且可区分的操作。  
**R-55** 对终端用户而言，“启用/关闭打造新兵器能力（工匠镜像）”必须作为一个独立、显式、可见的开关存在，而不是隐藏在某个高级开发者选项里。  

## 8. 各类 App 的适配要求

### 8.1 ScriptHost App

ScriptHost App 是本次升级的重点适配对象，要求如下：

1. 脚本开发者仍然只需面向统一的 Linux 容器运行环境编写应用，不需要自行处理三平台差异。
2. TypeScript（Deno）与 Python（uv）等依赖安装结果必须默认进入实例卷。
3. 应用运行期若发生自升级、下载 loader、生成本地工作副本等行为，这些内容必须落在实例卷。
4. 宿主机上的原始发布脚本仍可保持只读与可校验，但运行期真正执行的可变副本允许位于实例卷。
5. 删除实例卷后，ScriptHost App 需要能够重新拉起，并重建依赖与工作副本。

### 8.2 OpenDAN Agent App

OpenDAN 必须被纳入统一 App 模型，而不是继续维持独立特例：

1. Agent 的提示词、角色描述、任务规则、流程脚本等，应区分“上游发行版本”与“本地演化副本”。
2. 本地演化副本必须落在实例卷。
3. 升级逻辑应采用文件级同步/合并，不再依赖文件系统级 COW。
4. Agent 默认仍运行在受限容器边界内，其可见目录与宿主机能力继续由挂载策略/桥接工具决定。
   例外：Agent Service 原生运行（Desktop Lite、或 zone owner 把 Agent 放到某节点的 `host` 上）时不使用工作镜像。2026-11-04 起 Agent 的**私有 runtime 永远存在**：原生运行时它就是 Agent Service 进程自己的环境（`connector: native`），必须拥有 `$BUCKYOS_ROOT` 布局；`host` 只是登记 id，经 Node Executor 以节点 `owner_agents` 的 profile 执行（`notepads/opendan-agent-runtime-lifecycle-todo.md` §0.3、`doc/app_service/rust_app_loader.md` §9.3.1）。这种形态下 §6.4 表 B / 表 C 的工具由 `ext-core` 的原生包提供，发布前 `extended` 只是声明。
5. 删除实例卷后，Agent 应回退到更接近原始发行状态，这应被视为诊断与恢复能力的一部分。

### 8.3 传统二进制 App

对于原本就比较接近“镜像无状态 + 外部挂载”的纯二进制 App：

1. 若没有明显私有运行状态，可暂不强制写入大量实例数据。
2. 但框架层仍应支持为其创建实例卷，以便未来能力一致化。
3. 一旦应用引入插件、脚本、依赖缓存或本地可变规则，也应切换到统一实例卷模型。

### 8.4 工匠 Agent

不再本版本讨论

## 9. 推荐的逻辑目录划分

- BUCKYOS_ROOT=/opt/buckyos (固定)
- 环境变量契约:BUCKYOS_APP_ID, BUCKYOS_DATA_DIR, ...
- /tmp: tmpfs, 1GB, 容器销毁清空

### 代码层 (只读为主)
- /opt/buckyos/bin/$APP_ID/      # PKG_DIR, 建议视为只读,ScriptHost目录为可读写
  source: /mnt/buckyos/bin/$APP_ID/ (只读挂载)
  init: 首次启动 copy-on-init

### 工具层
- /opt/buckyos/tools/store/      # 全局只读,所有 App 共享
- /opt/buckyos/tools/bin/        # 按本 App 授权渲染的 symlink，创建App私有卷是构造
- /opt/buckyos/tools/deno-cache/ # OverlayFS lower
- /opt/buckyos/tools/uv-cache/   # OverlayFS lower

### 数据层
- /opt/buckyos/data/home/$userid/.local/share/$appid/             # 本 App 结构化数据,noexec
- /mnt/self/                     # 当前 zone CYFS 根, noexec
- /mnt/$zone_id/                 # 其他 zone CYFS 视图(按授权)
- /mnt/$node_id/                 # 特定设备的host fs数据(按授权)

### Agent 工具层(继承 App 视图,增加 session 层)
- /opt/buckyos/tools/$agentid/$sessionid/    # session-bin, 读写可执行


### Agent Root (数据层，和App语义不同)
- /opt/buckyos/data/home/$userid/.local/share/$agentid/            
  ├── sessions/$SESSION_ID/      # ssession 交付物
  ├── memory/                    # 跨 session 记忆
  └── skills/                    # 通用技能

和现有实现对比，主要是修改了sessions/<id>/.tool 的位置

### 权限矩阵
| 卷                      | 读 | 写 | 执行 |
|-------------------------|----|----|------|
| PKG_DIR                 | ✓  | ✓  | ✓    |
| tools/store             | ✓  | ✗  | ✓    |
| tools/bin (App)         | ✓  | ✗  | ✓    |
| tools/$sessionid(Agent) | ✓  | ✓  | ✓    |
| data/                   | ✓  | ✓  | ✗    |
| /mnt/<zone>             | ✓  | ✓* | ✗    |

## 10. 迁移要求

不用考虑向前兼容


## 11. 开发者声明要求（最少信息）

为支撑统一框架，应用声明中至少应具备以下信息：

1. 是否需要实例卷：`required / optional / disabled`
2. 实例卷推荐配额
4. 哪些路径属于外部业务数据挂载
6. 重置实例卷会丢失哪些内容

## 12. 验收标准

满足以下条件时，可认为本次升级达到目标：

1. ScriptHost App 在普通重启时不再重复安装主要依赖。
2. ScriptHost App 的运行期自修改行为被限制在实例卷内，而不是散落在宿主机不可控目录中。
3. OpenDAN 不再依赖文件系统级 COW 即可完成“上游更新 + 本地演化”的共存。
4. Windows、macOS、Linux 三类环境下的语义一致，不依赖平台特定文件系统行为。
5. 不会有Symlink指向宿主机的文件
5. 删除实例卷后，应用可以回退到接近原始发行状态，且外部业务数据不受影响。
6. 系统能以安全模式启动应用，支持诊断“卷损坏/本地演化异常”类问题。
7. App 间共享数据仅通过显式共享目录完成，不直接共享私有实例卷。
8. Agent 与普通 App 使用统一的安全模型与容器管理框架。
9. 系统提供独立的工作镜像与工匠镜像；在默认安装配置下，只有工作镜像被拉取与启动，工匠镜像不被拉取也不被启动。
10. §6.4 的三张表与 `STANDARD_TOOLS`、`EXTENDED_TOOLS`、`build_aios RUNTIME_PACKAGES`、`publish/exttool/` 一致，有测试或脚本检查守着；表里每条命令都有场景列。
11. 删掉 External Tool Volume 或任何扩展包后，表 A 的核心命令仍全部可用（核心不依赖扩展层）。



## 14. 结论

本次升级的本质，不是把容器变成“真正有状态”，而是把过去隐含、分散、跨平台脆弱的可变状态，收敛为一个明确的“应用实例私有层”；同时在镜像层引入“运行既有能力”与“打造新能力”的显式分层。

统一后的模型应满足以下原则：

- 镜像尽量无状态；
- 容器仍可重建；
- 私有可变状态进入实例卷；
- 业务/共享数据进入显式外部挂载；
- 升级与合并采用文件级策略；
- 权限扩大必须显式发生；
- Agent 与 App 必须回到统一安全模型；
- 工匠镜像不作为默认运行环境，也不作为交付物，它只生产独立可加载的工具包。

这将同时解决 ScriptHost 的依赖稳定性问题、OpenDAN 的自演化复杂度问题、Desktop/标准环境长期兼容与迁移问题，以及 Agent 自主扩展能力所带来的“依赖地狱”与“权限失控”风险，为后续的 App 生态与 Agent 生态统一奠定基础。