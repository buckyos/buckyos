# xllm 产品需求文档

- 产品名称：`xllm`
- 版本：v0.1 PRD 草案
- 日期：2026-09-17
- 定位：进入 AI 时代程序员工具箱的轻巧、可控、副作用小的 oneshot 命令行工具，提供 AICC 与 Agent 之间的任务执行能力
- 文档重点：使用者能完成什么任务、如何使用、得到什么结果，以及失败后如何继续

本文中的命令、参数和默认行为是新产品的设计要求，尚不表示已经实现。首版按本文重新定义用户体验，不承担旧 `run_local_llm` 命令的兼容要求。

## 1. 产品定位

**xllm 的目标是进入 AI 时代程序员的工具箱，让 LLM 能力成为各种小脚本中可以频繁使用的一个步骤：轻巧、可控、副作用小。** 每次新任务都从干净的 LLM context 开始，处理本次问题和材料，完成后输出结果并退出。提取字段、归类文本、总结日志、转换内容和生成简短说明都是主要用法。

xllm 刻意把边界放在 AICC 与 Agent 之间：在模型推理之上提供输入组装、通用执行循环、可选工具、执行限制和 Run 恢复；跨任务的目标、流程编排、身份和长期状态由调用方管理。一个 Run 可以包含多次模型调用，但完成后不再追加新问题。这一层长期独立存在，既供 shell 脚本使用，也供上层 Agent 调用。

简单问题可以直接得到回答，需要操作本地文件或外部服务的任务可以启用工具。本次 context 由本次输入和生效配置构造，不继承其它 Run 的对话、工具记录或隐含会话状态。需要处理已有结果时，用户通过文件或管道显式提交所需材料，发起新的独立任务；只有恢复同一次未完成任务时，才沿用该 Run 保存的上下文。使用者不需要先创建 Agent 或维护 session；可选择 function call 或 behavior 执行循环，无需定义多个 behavior 或切换状态机。

产品的主要价值：

1. **轻巧**：配置好模型访问后即可直接调用，不要求启动 Agent 会话或额外的 xllm 常驻进程。简单任务直接完成，不强制规划、工具调用或多阶段报告；常用提示词可以复用。
2. **可控**：输入、有效配置、模型、工具开关、输出形式和执行限制都有明确含义；脚本能根据退出状态继续或停止，命令不等待人工对话。
3. **副作用小**：未配置时关闭模型主动工具调用；默认本地写入限于 Runs 记录，显式指定的输出文件按要求保存。模型自行读取其它业务文件、改写文件或执行命令需要启用工具，不自动修改目录配置、维护长期记忆或启动后续后台工作。
4. **接得起来**：stdout 可直接交给变量、文件或下一个命令，stderr 提供诊断；脚本负责分支、循环和并行调度，各次调用不依赖隐含会话状态。
5. **失败可恢复**：中断或可恢复错误保留本次任务进度，调用方决定何时 resume；恢复不引入新问题，也不把任务失败变成无限等待。

首版交付独立的 `xllm` 命令，并由同一 SDK 提供其核心能力。SDK 用户可以直接调用任务执行能力，无需启动 CLI 子进程或解析终端文本。本文不规定 SDK 类型、存储 schema 或执行引擎的实现方式。

## 2. 目标使用者与主要场景

主要使用者是程序员和脚本编写者。首先满足日常命令、管道、循环和已有自动化流程中的小任务，再通过显式启用工具覆盖需要操作本地环境的任务。

| 使用者 | 想完成的事情 | xllm 应提供的体验 |
| --- | --- | --- |
| 程序员与脚本编写者 | 提取字段、分类、总结或转换一份材料，嵌入已有脚本 | 短命令、标准输入输出、结构化结果、明确的成功和失败判断 |
| 开发者 | 分析日志、比较截图、整理代码目录、生成报告 | 与现有目录和命令组合使用，按需开启工具，知道产物保存在哪里 |
| 熟悉终端的普通用户 | 问问题、读图、总结一份文本 | 一条命令提交材料和问题，直接看到回答 |
| Agent 或 SDK 集成者 | 委托一个有输入、有边界、有结果的任务 | CLI 与 SDK 对同一请求提供一致的任务行为 |

典型任务包括：

- “从构建日志中提取错误类别和原因，返回 JSON。”
- “把这段变更说明压缩为一句话，供脚本保存。”
- “将这条反馈归类为 bug、feature 或 question，只输出类别。”
- “总结这段日志的异常原因。”
- “这张图里有什么？”
- “比较这两张截图，指出界面变化。”
- “读取这个目录中的文档，生成一份索引。”
- “把这份分析报告整理成表格。”
- “刚才被中断了，从保存的进度继续。”

## 3. 使用者需要理解的概念

### 3.1 工作目录

工作目录默认直接使用启动 xllm 的当前 Bash 的 `CWD`。日常使用时，用户先 `cd ./project`，再直接运行 `xllm`、`xllm list` 等命令，无需反复传入目录。需要在不切换当前目录的情况下操作其它位置时，可用 `--dir` 显式指定工作目录。任务启用工具后，工具默认从工作目录开始操作，生成文件也默认落在这里；命令查找直接使用从当前 Bash 继承的 `PATH`。

工作目录与任务记录目录（Runs 目录）分开：Runs 目录可通过 `.llm_context` 配置，也可由 `--runs-dir` 覆盖；未配置时使用 `~/.xllm/runs`。

- 同一个工作目录可以保留多次任务。
- 新任务不会清空目录或覆盖旧任务记录；同一工作目录不代表共享会话，每次新任务都独立构造 LLM context。
- 同一个 Runs 目录可以保存多个工作目录的任务，每条记录保留原工作目录；更换 Runs 目录不会自动迁移已有记录。
- 删除某次任务记录会清除它的历史与恢复材料，不会替用户删除已经生成的业务文件。
- 执行任务时 Runs 目录需要可写；工作目录按任务是否需要写入产物检查权限。不满足时，在发起模型请求前说明具体路径和原因。

### 3.2 一次任务

每次新发起的工作称为一次 Run，都有唯一任务编号 `runid`，可能包含多次模型调用及工具操作。oneshot 的边界是一次独立任务；同一 Run 内可以积累完成该任务所需的模型响应和工具结果，任务结束后不再接收下一轮问题。编号用于查看结果、关联日志和判断正在恢复哪次任务，用户不必理解内部快照结构。

标准 loop 是通用的任务执行循环：模型根据任务目标、已有材料和执行结果，决定直接回答、继续分析、调用工具、检查结果或调整做法，直到交付结果或触发停止条件。`loop_model` 支持 `function_call`（默认）和 `behavior`，分别使用模型原生工具调用或行为协议驱动执行；二者遵守相同的任务、限制和恢复语义。问答、总结、写作、代码修改和文件整理共用这一框架；每次任务可以采用不同步骤，不要求先规划、必须调用工具或固定执行若干阶段。关闭工具时只进行一次任务推理，不进入工具或动作循环。提示词组可以同时携带任务要求、system section 和工具配置，无需为每类任务另写一个 loop。

**本次命令失败，不等于任务已经终止。** 一次 Run 可以跨多次 CLI 调用继续执行：Provider 超时、限流、临时不可用等可恢复错误让本次命令非零退出，但保存任务进度，下次 resume 仍可继续尝试。

| 任务状态 | 是否终态 | resume 行为 |
| --- | --- | --- |
| 执行中 | 否 | 不重复启动正在执行的任务 |
| 已中断 | 否 | 有有效进度时继续执行 |
| 暂停（可恢复错误） | 否 | 从保存进度继续，重新尝试未完成的请求或步骤 |
| 已完成 | 是 | 只展示最终结果 |
| 失败（不可恢复）或达到限制 | 是 | 只展示最终结果与终止原因 |

一旦进入终止状态，同一 `runid` 不得再次执行。Ctrl-C、可恢复错误和进程退出本身不等于任务已到终态；可恢复错误重复发生，也不能仅因失败次数增加就把任务永久终止。执行限制与不可恢复错误仍按相应规则处理。

### 3.3 新任务与恢复

| 用户意图 | 操作 | 预期 |
| --- | --- | --- |
| 开始一件独立的事 | `xllm "问题"`，或沿用 `--user` | 新任务、干净 context，只使用本次输入、配置及任务中读取的材料 |
| 运行一个常用任务 | `xllm --select fixissue` | 选择已有提示词组，使用组内的默认任务要求 |
| 执行目录默认任务 | 配置默认组和默认任务后直接运行 `xllm` | 使用该目录选中的组及其工具配置创建新 Run |
| 处理已有结果 | 通过 `--file` 或 stdin 显式提交材料，并给出新要求 | 新建独立任务，只传递选中的材料，不带入原任务的对话与工具记录 |
| 继续中断或可恢复错误暂停的工作 | `--resume`，可配合 `--run <id>` | 恢复保存的任务进度，重试未完成步骤，不再要求重输问题或附件 |
| 找到最近的任务编号 | `xllm list` | 查看最近多次任务的编号、概要、状态和工作目录 |

重复执行一条普通命令表示再次发起任务，使用新的 context，不能静默变成恢复或接续上次对话。resume 仅继续同一次未完成任务，不用于给已有任务增加新要求。

### 3.4 目录配置与提示词复用

`.llm_context` 是面向用户的目录配置文件。xllm 从工作目录开始向父目录逐级查找，直到文件系统根目录，将找到的配置按“最远父目录到当前工作目录”的顺序合并。子目录可以只写差异项；命令行显式参数的优先级高于文件配置。

配置覆盖 Provider 接入方式、模型、执行循环、提示词、工具与动作、日志级别、结果提取和 Runs 目录等执行设置。父目录可以维护多个命名提示词组，组既可内联定义，也可引用独立 YAML 文件；子目录选择一个组，再定制其中的 section 或工具，命令行也可通过 `--select <name>` 选择组。组可以附带默认任务要求；配置默认组后可直接运行 `xllm` 执行该任务。标准模式的每个 system section 都有行号，按行号升序拼接；系统为部分行号提供固定别名，用户可通过指定其它行号插入自己的 section，并通过模板变量填入运行环境。需要完全控制业务提示词时，也可以配置整段自定义内容。执行循环依赖的协议提示词由系统管理，不属于可替换的业务内容。

新任务先按 **当前工作目录配置 > 逐级父目录配置 > 模型访问环境与产品默认值** 合并各层同名配置，再选组并应用组内配置，最后应用显式 CLI 参数；CLI 优先级最高。工具的顶层默认、组内覆盖和目录级覆盖关系见 F04，section 的组装关系见 F11。目录配置和提示词组不保存或累积会话历史。恢复任务对已保存配置的处理见 F05。

## 4. 核心使用体验

以下示例描述目标体验。模型访问环境已配置，示例引用的文件和模型均需要实际可用。

### 4.1 一句话得到回答

```bash
./xllm "sanjose 今天天气如何"
```

一句话直接作为位置参数，不必先写 `--user`；安装到 PATH 后可直接写 `xllm "问题"`。原有 `xllm --user "问题"` 保留为等价的显式写法。天气示例表达的是输入体验；真实天气回答需要配置可用的实时数据来源，无法取得时应说明，不能凭模型记忆编造。

常用任务也应能用短命令启动：

```bash
./xllm --select fixissue
```

`fixissue` 复用 `.llm_context` 中定义的提示词组。组内配置默认任务要求后，用户不必重复输入；也可用 `xllm --select fixissue "修复 issue-123.md 中的问题"` 替换本次任务要求。若目录已配置 `prompt.select: fixissue`，直接运行 `xllm` 即可执行默认任务。组不存在，或既没有默认任务也没有其它有效输入时，应明确提示需要补充什么，不发起空任务。

默认向 stdout 输出最终回答。进度、任务编号、用量等辅助信息写入 stderr，不夹在答案中。

未配置工具开关时，仅处理用户显式提交的材料。需要模型自行调用本地工具时，使用 `--tools`，或在 `.llm_context` 中预先启用；例如需要读取项目并修改文件的 `fixissue` 可在目录配置中启用工具。`--no-tools` 始终可以显式关闭。

### 4.2 一张图片和一句话

```bash
xllm --image ./photo.png --user '这张图片里有什么？'
```

用户直接给本地文件，xllm 负责准备图片输入。无需上传到公共网址，无需手工编码，也无需先运行另一个媒体分析工具。

图片和问题属于同一次提交，负责图片理解的模型应看到真实图片内容。配置 `file_model` 时，它结合本次问题分析图片，将分析结果交给 `model` 完成主任务；未配置时由具有视觉能力的主模型直接处理。不能只把文件名作为文字发送后声称已经完成看图。

多图比较：

```bash
xllm \
  --image ./before.png \
  --image ./after.png \
  --user '第一张是修改前，第二张是修改后。请列出变化。'
```

支持直接指定可访问的图片 URL：

```bash
xllm --image 'https://example.com/photo.png' --user '描述图片中的主要物体。'
```

图片的传入顺序必须保留，“第一张”“第二张”能对应用户指定的顺序。

### 4.3 从文件或管道提交材料

```bash
xllm --file ./error.log --user '分析异常原因，并列出排查顺序。'
```

```bash
git diff | xllm '评审这份变更，重点检查可能的行为回退。'
```

用户不需要把长文本粘贴进命令行。文件名作为材料标签保留，正文作为输入处理；有本次问题或组默认任务时，管道内容作为补充材料。没有其它任务要求时，非空 stdin 本身作为 user 输入，因此也支持 `printf '解释事件驱动架构\n' | xllm`。

首版 `--file` 处理文本文件。图片使用 `--image`；PDF、音视频等其它类型不应被默默当作文本或文件名处理。

### 4.4 在目录中完成任务并生成文件

```bash
cd ./project
xllm --tools '阅读 docs 目录，生成 docs-index.md，列出每份文档的主题。'
```

启用工具后，xllm 可以自行读取材料、执行可用命令、写入或修改文件，并在必要时多次调用模型完成任务。

用户最终获得：任务结论、已知产物的路径，以及任务是否正常完成。进度中能看出正在使用哪项工具；某次工具调用失败而随后被纠正，不应直接把整个任务标为失败。

首版复用新的 TS agent tools 提供的工具能力。用户已有的可执行命令直接通过当前 Bash 的 `PATH` 查找并调用，不要求每个脚本都先改造成专用 API。

### 4.5 恢复同一次未完成任务

中断或遇到可恢复错误后，可在项目目录中查看状态并恢复。例如新开终端后，从项目的父目录进入：

```bash
cd ./project
xllm list
xllm status --run "$runid"
xllm --resume --run "$runid"
```

其中 `$runid` 来自任务启动/中断时的提示或 `xllm list`。也可直接运行 `xllm --resume`，选择当前工作目录最近的未完成任务。进入项目目录后会自动查找 `.llm_context`，包括其中配置的 Runs 目录。按编号恢复时读取原工作目录与保存的配置；从其它位置访问记录时，可用 `--dir` 指定配置所在的工作目录，或用 `--runs-dir` 显式指定记录位置。

例如 Provider 暂时不可用时，本次命令非零退出并告知 runid；服务恢复后执行同一条 resume 命令，继续未完成的模型请求。若仍不可用，本次恢复再次失败退出，任务仍保持可恢复，可在后续再次 resume，直到完成或遇到真正的终止条件。

用户不需要手工读取或修改任务记录。指定编号不存在或没有有效恢复进度时，应明确说明，不能偷偷创建一个空任务。指定任务已经正常完成、因不可恢复错误失败或达到限制时，只展示已保存的最终结果，不再调用模型或工具。

### 4.6 将结果交给脚本

让模型返回 JSON 内容：

```bash
xllm --file ./error.log --user '提取 error_count 和 main_cause 两个字段。' --json
```

获取包含任务状态、答案、产物和用量的结构化执行结果：

```bash
xllm --file ./error.log --user '分析异常原因。' --format json
```

两者可以组合：`--json` 约束答案内容，`--format json` 选择整个 CLI 执行结果的表示方式。

作为小脚本中的一步，直接用退出状态控制后续操作：

```bash
if summary=$(xllm --no-tools --timeout 60 --file ./build.log '用一句话总结构建结果。'); then
  printf '%s\n' "$summary" > ./build-summary.txt
else
  exit 1
fi
```

这里 `--no-tools` 显式覆盖目录配置，确保模型只处理提交的材料；脚本负责保存答案和失败分支。xllm 自行保存的 Run 记录位于 Runs 目录，不在项目里创建 Agent 工作区。调用方可以在循环或并行脚本中重复使用同样的命令，每次输入、结果与 runid 独立。

保存最终输出：

```bash
xllm --file ./notes.txt --user '整理成 Markdown 摘要。' --output ./summary.md
```

`--output` 保存与所选输出格式对应的最终结果。成功后不再向 stdout 重复输出正文，stderr 告知保存位置。

一个 xllm 的答案可以直接作为另一个 xllm 的材料：

```bash
xllm --file ./error.log '总结异常和已知事实。' |
  xllm '根据输入，列出优先级最高的三项排查步骤。'
```

也可以串联已有提示词组：

```bash
git diff | xllm --select review | xllm '将评审意见整理为待办清单。'
```

这里 `review` 组预先配置了默认评审任务。串联复用 stdout/stdin，不需要中间文件、复制粘贴或读取内部 Run 记录；下游等待输入完整接收后，以干净 context 开始新任务，每个 xllm 保留独立 runid。下游只接收管道中的答案，不继承上游的对话和工具记录。`--json` 输出的 JSON 答案也能直接作为下游文本材料，保留其内容。

默认文本模式失败时，错误写入 stderr，不把错误或半份答案当成上游结果发送；下游收到空管道时明确失败，不凭自己的提示词启动任务。`--format json` 包含完整执行状态及错误信息，直接接入管道时也只作为普通文本材料，不自动提取答案或恢复任务；需要按成功状态分支时，由脚本先检查上游退出状态和结构化结果。

### 4.7 父目录复用配置，子目录定制任务

例如，一个项目可以按以下方式组织配置；完整 YAML 示例见 4.8，正式文件语法由后续 CLI/SDK 协议固定：

| 配置位置 | 配置内容 | 生效效果 |
| --- | --- | --- |
| `project/.llm_context` | Provider、模型、loop、`review` 与 `docs` 提示词组及其工具、外部 `checkpr` 组、Runs 目录 | 向所有子目录提供可复用的配置与任务 |
| `project/src/.llm_context` | 选择 `review` 组，定制 `rules`（30），并在 25 行插入项目约定 section | 继承该组其余 section，在环境上下文与执行规则之间加入本目录的项目约定 |
| `project/docs/.llm_context` | 选择 `docs` 组，定制 `output_format`（100） | 复用文档任务指令，输出适合该目录的文档格式 |

```bash
cd ./project/src
xllm '评审当前目录中的代码。'
cd ../docs
xllm --model "$model_name" '整理当前文档的主要问题。'
```

切换到不同目录后，xllm 从当前目录向上查找并合并配置。其中 `$model_name` 为本次要使用的模型名。第二次 xllm 调用的模型参数覆盖 `.llm_context` 中的模型设置；其它配置继续继承。某个目录也可以改为完整自定义提示词模式，此时业务提示词直接使用配置的整段内容，不再拼接标准 section 或提示词组；运行时必需的协议提示词仍由系统提供。

### 4.8 完整的 `.llm_context` 配置示例（YAML）

以下示例表达首版配置意图：Provider 接入、主模型与文件模型、执行循环、日志与结果输出由目录配置统一指定；提示词组同时携带任务、system section 和工具。YAML 字段用于明确需求，正式解析规则由后续 CLI/SDK 协议落实，不表示当前命令已支持该格式。示例假设 BuckyOS 服务和逻辑模型可用，session_token 已替换为有效身份；MCP 地址、sendmsg 工具及 rg/git 命令也需要实际可用。

`project/.llm_context`：

```yaml
# Provider 接入方式：默认 buckyos，也支持 openai。
# buckyos 使用 AICC、taskmgr 等服务；openai 使用自己的连接与凭据配置。
provider:
  type: buckyos
  # 可省略以沿用当前身份；手工配置时将占位内容替换为有效 token。
  session_token: "<BuckyOS session token>"

# 主模型负责完成任务；文件模型先理解图片，再把分析结果交给主模型。
# 未配置 file_model 时，主模型需直接支持图片；纯文本任务不调用文件模型。
model: llm.chat
file_model: llm.vision
# 单次模型输出 token 上限，需实际模型支持；不是上下文窗口大小。
max_tokens: 256000
# 工具轮数上限，原生 tools 与 behavior actions 共用此限制。
max_rounds: 8
# 单次 LLM 请求超时，单位秒；--timeout 另行限制整条命令的总时长。
llm_timeout: 600
# Run 记录和恢复材料的位置；相对路径以本配置文件所在目录为基准。
# 如果配置为None 说明不使用磁盘保存状态（适合在无磁盘写入权限的设备上只读运行）
runs_dir: "~/.xllm/runs"


# 默认 function_call（原生工具调用）；behavior 使用行为协议推进任务。
loop_model: behavior
# 过程日志只写 stderr；详略依次为 debug、info、warn、result，默认 info。
# result 隐藏常规进度，但仍保留失败/中断所必需的诊断。
run_logs: info
# 默认 raw：保留最终模型响应原文。result.report：提取 JSON/XML 中的 report。
# result 是最终响应的根节点别名；提取规则也作用于 > result.out 和管道。
# 此配置须与下方 output_format 对应；--format json 再包装 CLI 执行结果。
result_format: result.report

prompt:
  # standard 按行号组装 sections；custom 使用整段 system，见后面的替换示例。
  mode: standard
  # 默认组有 default_user 时，进入本目录直接运行 xllm 即可执行。
  # --select 可换组；命令行问题可替换组内默认任务。
  select: review
  groups:
    # 外部 YAML 的根节点是一个组对象；路径相对于本配置文件。
    checkpr: "./checkpr.yaml"
    review:
      # 本次任务要求，进入 user 输入；sections 中的内容进入 system。
      default_user: |
        评审当前工作目录中的代码，找出影响正确性的主要问题。
        给出依据和修改建议，不直接修改文件。
      sections:
        # 固定别名映射到行号；按行号升序拼接，不按 YAML 的书写顺序。
        role: # 10：角色与职责
          text: |
            你是代码评审助手，重点检查正确性、边界条件和错误处理。
        # runtime 变量由系统提供；env.PROJECT_NAME 需在运行前设置。
        # 只渲染选中组的生效文本；缺失变量报错，resume 沿用原渲染结果。
        contexts: # 20：上下文与环境，也可用别名 env
          text: |
            项目名称：{{env.PROJECT_NAME}}。本项目是 Rust 服务，错误通过 Result 返回。
            当前时间：{{runtime.current_time}}；时区：{{runtime.timezone}}。
            操作系统：{{runtime.os}}；工作目录：{{runtime.cwd}}。
        # 自定义行号：在 20 与 30 之间插入，不必重编号其它 section。
        "25":
          name: project_conventions
          text: |
            公共接口变更需要检查调用方，错误信息应提供足够的定位线索。
        rules: # 30：行为规则；系统另行填入实际可用 tools/actions 的说明
          text: |
            先读取相关实现和调用方，再给出结论。
            只依据实际读取的材料判断问题，区分已验证事实与推测。
            工具使用范围以系统提供的本次可用工具和 actions 说明为准。
        cmd_manual: # 40：exec 命令手册；这里补充偏好，具体命令说明来自 bash_tools
          text: |
            搜索代码时优先使用 rg，查看变更时使用 git diff。
            命令和参数以系统生成的 exec 手册为准。
        output_format: # 100：最终内容要求；与上面的 result.report 提取方式配套
          text: |
            按 behavior 协议提交最终结果，在 report 字段中用中文列出评审结论。
            按严重程度列出问题，每项包含文件位置、依据和建议；没有发现问题时直接说明。
      # 工具属于组配置：仅选中本组时生效，可覆盖顶层 tools 基础配置。
      # prompt.tools 可进一步覆盖组配置，CLI 的 --no-tools 优先级最高。
      tools:
        # 默认关闭；false 同时关闭 tools/actions，主任务只进行一次推理。
        # 显式附件的文件模型分析属于输入准备，仍可能单独调用。
        enabled: true
        # 仅适用于 behavior：把 tools 转为 actions，原生 tools 列表置空。
        # 默认 false；behavior 也可分别配置 tools 与 actions，两者可并存。
        tools2actions: true
        # 三种来源分别为内置工具组、MCP 服务和单个已注册内置工具。
        tools:
          - groupname: bash # 包含 read_file、write_file、exec
          - mcp: "http://127.0.0.1:3000/mcp" # 示例地址，需替换为实际可用服务
          - name: sendmsg # 不属于 bash 组，需运行环境提供
        # exec 的补充命令手册；不会安装命令或单独启用 exec。
        bash_tools:
          - name: rg
            description: 搜索工作目录中的源码与文档。
            command: rg
            usage: "rg --line-number -- <pattern> <path>"
          - name: git_diff
            description: 查看尚未暂存的 Git 变更。
            command: git
            usage: "git diff -- <path>"
    # 组之间不相互继承；docs 单独声明读取文档所需的 bash 工具。
    docs:
      default_user: |
        阅读当前工作目录中的文档，整理主要问题和改进建议，不直接修改文件。
      sections:
        role:
          text: 你是技术文档编辑，关注准确性、结构和示例是否易于理解。
        rules:
          text: 区分文档中已有的事实与仍需验证的内容，不编造接口或功能。
        output_format:
          text: 在最终结果的 report 字段中，用 Markdown 表格列出文档位置、问题和建议。
      tools:
        enabled: true
        tools2actions: true
        tools:
          - groupname: bash
  # 目录级 section 对所有选中组生效：同一行号覆盖，新行号插入。
  sections:
    "50":
      name: delivery_checklist
      text: |
        交付前检查文件引用是否准确，并说明尚未验证的内容。
```

`provider.type` 默认 buckyos，也可选 openai；buckyos 会使用 AICC、taskmgr 等所需服务，`session_token` 可手工指定，也可省略以沿用已有身份。`model` 执行主任务，`file_model` 处理图片等首版支持的附件理解；没有附件时不调用文件模型。`max_tokens` 是单次输出上限，256000 仅适用于支持该值的实际模型；`max_rounds` 限制工具轮数，`llm_timeout` 限制单次 LLM 请求等待秒数，与 `--timeout` 的命令总时长分开。`runs_dir` 的相对路径始终相对于声明它的配置文件。

`loop_model` 支持 function_call（默认）和 behavior。示例选择 behavior，并在 review 组中用 `tools2actions: true` 把 bash 组、MCP 工具和 sendmsg 统一转换为 actions，原生 tools 列表变为空；也可关闭转换并分别配置 tools 与 actions。`bash_tools` 只补充 exec 命令手册。组内工具可覆盖顶层 tools 基础配置，`prompt.tools` 可进一步提供目录级覆盖，`--no-tools` 最后覆盖所有组和目录开关。未选中的组不提供任何工具；docs 组因此单独声明自己需要的 bash 能力。

`run_logs: info` 将阶段进度写入 stderr；可选 debug、info、warn、result 四档，不改变 stdout 的结果。`result_format: result.report` 从模型最终结构化响应中提取 report；`result` 是解析后的根节点别名。改为 raw 则保留最终模型响应原文。这里的 output_format 提示词也明确要求 report 内容，使 `xllm > result.out` 可以直接得到报告；`--format json` 再将提取结果包装为 CLI 执行结果。

外部组文件 `project/checkpr.yaml` 的根节点直接是一个组对象，可写为：

```yaml
# 文件：project/checkpr.yaml。直接定义组内容，无需再包 prompt/groups。
default_user: |
  检查当前工作目录的 Git 变更，说明正确性问题和需要补充的验证，不修改文件。
sections:
  role:
    text: 你是变更评审助手，依据实际 diff 和相关代码判断问题。
  output_format:
    # 主配置选择 result.report，因此本组也将结论放入 report。
    text: 在最终结果的 report 字段中用中文列出问题、依据和建议。
tools:
  enabled: true
  # 使用主配置的 behavior loop，将下面的 bash 工具转为 actions。
  tools2actions: true
  tools:
    - groupname: bash
```

`prompt.groups` 定义组，`prompt.select` 选择默认组；`default_user` 进入 user 输入，组内 sections 进入 system。`prompt.sections` 对所选组提供目录级覆盖或新增内容。section 的键可以是固定别名或带引号的数字行号，text 为用户文本，name 为可选自定义名称；同一集合不能重复声明同一行号及其别名。

默认 review 的 system 顺序为 **10 role → 20 contexts/env → 25 project_conventions → 30 rules → 40 cmd_manual → 50 delivery_checklist → 100 output_format**，最后追加系统的 runtime_protocol。模板在组选取和文本合并后渲染；系统同时保证环境信息、可用 tools/actions 与 exec 手册和实际配置一致。切换到 docs 或 checkpr 后，不再带入 review 的文本、MCP 或 sendmsg 配置，目录级 50 行仍生效。

`project/src/.llm_context` 可以只写差异：覆盖规则和项目约定，清空 50 行，用行号 100 覆盖 output_format，并将所选组的工具来源列表替换为 bash 组。

```yaml
# 文件：project/src/.llm_context。未声明的 Provider、模型、loop 等继续继承。
prompt:
  select: review
  sections:
    # 覆盖 30 行的用户文本；系统生成的实际能力说明仍保留。
    rules:
      text: 先检查调用方和错误路径；只报告能够给出具体代码依据的问题。
    "25":
      name: project_conventions
      text: 本目录包含公共接口，实现变更需检查调用方对返回值的处理。
    "50":
      # 空字符串显式清空继承内容；该 section 最终为空时连标题一起省略。
      text: ""
    # 100 与 output_format 是同一位置，覆盖后只输出一次。
    "100":
      text: 在最终结果的 report 字段中用中文列出问题、代码位置、依据和验证步骤。
  tools:
    # 列表整体替换：只保留 bash，不继承组内的 MCP/sendmsg。
    # enabled 和 tools2actions 未声明，继续沿用 review 组的值。
    tools:
      - groupname: bash
```

该子目录继承 Provider、模型、loop、组定义及组内的 enabled/tools2actions，只替换工具来源列表；最终只暴露由 bash 转换的 actions，section 行号为 **10、20、25、30、40、100**。假设从 project 的父目录开始，且示例文件和环境已准备好：

```bash
export PROJECT_NAME=example-project
cd ./project
xllm
xllm --select checkpr
xllm --select docs '检查 README.md 的结构和示例。'
cd ./src
xllm
xllm --no-tools --file ./lib.rs '仅评审这份文件中的错误处理。'
```

直接运行 xllm 会执行默认 review 任务；重复调用仍创建独立 Run。`--select` 显式换组，问题参数替换组默认任务。最后一条命令关闭 tools 和 actions，只处理提交的文件并进行一次主任务推理；模板中的 PROJECT_NAME 需要已设置，否则在模型调用前报错。

完整自定义业务提示词可改用以下配置片段：替换整个 prompt 块，并将 result_format 改为 raw；保留其余 Provider、模型和限制配置。此时没有选中组，不继承 review 的任务或工具，通过命令行显式给出本次问题。

```yaml
# 替换主配置中的 result_format 和整个 prompt 块；其余顶层配置保留。
# raw 保留最终响应原文，不再要求提取 report 字段。
result_format: raw
prompt:
  # 不与 select/sections 同时配置；不再继承任何组的默认任务或工具。
  mode: custom
  # 整段业务 system 也支持模板；运行时协议仍由系统提供。
  system: |
    你是日志分析助手，当前工作目录为 {{runtime.cwd}}。
    根据本次提交的材料解释错误原因，区分已知事实和推测，用中文简洁回答。
```

mode: custom 不同时声明 select 或 sections；工具按顶层 tools、prompt.tools 和 CLI 计算，运行时协议及实际能力声明继续由系统提供。

### 4.9 通用配置模板（参考 pi-mono）

本节提供适用于当前 xllm 的通用 `.llm_context` 配置模板。适用于日常问答、阅读项目、修改代码、整理文档和执行本地命令；同一份配置通过每次传入的任务要求复用。

提示词结构参考 pi-mono 的 [system-prompt.ts](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/system-prompt.ts)：简短角色说明、实际可用工具、操作规则、项目上下文与工作目录；文件操作参考其 [read](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/tools/read.ts)、[edit](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/tools/edit.ts)、[write](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/tools/write.ts) 的职责划分。下面按 xllm 的工具名称和单次任务语义改写，配置字段以当前 [Rust SDK 实现](../../src/frame/agent_tool/src/local_llm_context.rs) 为准。

#### 4.9.1 可直接复用的 `.llm_context`

将以下内容保存为项目根目录的 `.llm_context`。示例使用 BuckyOS 已有身份和 `llm.chat` 模型别名；使用前确认该别名在当前 AICC 环境可用并支持原生工具调用，或替换为实际模型。`max_tokens` 是单次输出上限，需在所选模型的支持范围内。

```yaml
provider:
  type: buckyos
model: llm.chat
loop_model: function_call
max_rounds: 16
llm_timeout: 600
timeout: 3600
runs_dir: "~/.xllm/runs"
run_logs: info
result_format: raw

prompt:
  mode: standard
  select: general
  groups:
    general:
      sections:
        role:
          text: |
            You are a general-purpose task assistant running inside xllm. Help users
            understand information, work with files, edit code, write documentation,
            and complete command-line tasks. Deliver a usable result for the current request.
        contexts:
          text: |
            Current time: {{runtime.current_time}}
            Timezone: {{runtime.timezone}}
            Operating system: {{runtime.os}}
            Working directory: {{runtime.cwd}}
            Resolve relative paths from this run's working directory.
            Do not assume there is an additional workspace subdirectory.
        rules:
          text: |
            - Answer simple questions directly. Use the tools available in this run when
              the task requires inspecting, changing, or verifying the environment.
            - When tools are available and the task involves a project, first read the
              applicable AGENTS.md, relevant README files, and target files. Follow project
              conventions. If the project provides skills, read the relevant SKILL.md only
              when needed; do not assume its contents have already been loaded.
            - Understand the relevant implementation and inputs before making changes.
              Prefer existing code, dependencies, scripts, and file structures.
            - Make only the changes needed for the task and preserve the user's existing
              changes. Do not commit, push, or publish without authorization.
            - These file-operation rules apply only to tools enabled in this run; follow
              their actual parameter schemas. Use read_file to inspect files and edit_file
              for targeted replacements; old_string must match the original text exactly
              and uniquely. Use write_file for new files or necessary complete rewrites.
            - Use exec for directory listings, searches, builds, tests, and scripts.
              Do not assume cd or variable assignments persist between commands.
              Do not attempt to execute commands when exec is disabled.
            - Read files and command output as needed, focusing on relevant ranges.
              If output is truncated, retrieve the missing parts before drawing conclusions.
            - When a tool fails, read the error and adjust the arguments or approach.
              Avoid repeating an unchanged failing operation; use results to guide the next step.
            - After making changes, run relevant checks and inspect the relevant diff when
              Git is available. Clearly state which checks were not run.
            - This is an independent task. Do not rely on a previous Run's conversation or
              initiate interactions that require waiting for a user reply. For minor missing
              details, make and state reasonable assumptions. If essential input is missing,
              deliver completed work and clearly explain what blocks further progress.
            - Ground conclusions in the actual materials and tool results. Distinguish
              verified facts, assumptions, and unverified claims. Never invent execution results.
        cmd_manual:
          text: |
            Prefer rg and rg --files for text and file searches. If unavailable, use
            installed alternatives such as grep and find.
            In Git projects, use git status --short and git diff to inspect changes.
            Look up build and test commands in the project documentation.
            Quote path arguments correctly. Narrow large searches before reading their
            output; do not dump the entire project just to browse its files.
        output_format:
          text: |
            Respond in the user's language. Lead with the result, stay concise, and follow
            any output format specified by the user.
            Show file paths clearly and include line numbers when useful. For changes,
            describe what changed, the verification results, and any unfinished work.
            Use plain text or Markdown by default. When the runtime requires JSON, output
            only valid JSON without surrounding explanations or code fences.
      tools:
        enabled: true
        filesystem_policy: unrestricted
        tools:
          - groupname: bash
```

这组配置采用 `function_call + raw`：工具调用通过原生接口完成，最终 assistant 文本直接交付，无需手写 XML 或 `report` 包装。内置 `bash` 组实际提供 `read_file`、`edit_file`、`write_file`、`exec`，对应 pi-mono 常用的 `read`、`edit`、`write`、`bash` 分工；不要把 pi 的工具名或参数直接用于 xllm。

`role / contexts / rules / cmd_manual / output_format` 分别映射到行号 10 / 20 / 30 / 40 / 100。SDK 会补充本次实际工具说明和 `runtime_protocol`；模板只填写业务规则。项目约定和技能文件需要模型按需读取或由调用方显式提供，xllm 不因上述提示词自动发现、注入这些文件。

#### 4.9.2 使用与最常见的调整

在保存配置的项目目录中运行：

```bash
agent_tool xllm '阅读项目入口和 README，说明主要模块及启动方式。'
agent_tool xllm '修复当前失败的单元测试，尽量保持修改范围小，并运行相关测试。'
agent_tool xllm '检查 docs 中的失效相对链接，修复后报告修改的文件。'
git diff --no-ext-diff | agent_tool xllm --no-tools '评审输入的 diff，指出有依据的问题。'
agent_tool xllm --no-tools --json '返回一个包含 summary 和 items 的 JSON 对象，概括：配置、执行、验证。'
```

- **任务与工作目录**：模板不设 `default_user`，每次通过问题、`--user` 或非空 stdin 提交任务；只有配置而没有任务时显示用法。`--dir /absolute/project` 可指定工作目录，配置会从该目录向上查找并合并。每条普通调用创建独立 Run。
- **工具与文件权限**：模板显式设置 `tools.filesystem_policy: unrestricted`，内置文件工具可读写工作目录外的路径，`exec.cwd` 也可指向其它目录，实际访问由运行 xllm 用户的操作系统权限决定。工作目录仅作为默认位置和相对路径基准。省略该字段时为 `workspace`，会限制文件工具路径和 `exec.cwd`；需要自由访问时保留模板中的 `unrestricted`。`--no-tools` 仍可关闭所有工具。该策略只控制内置 `bash` 组，MCP 和宿主注入工具遵循各自权限规则。
- **预算与输出**：16 是本模板选择的工具轮数上限，不是 SDK 默认值；`llm_timeout` 与 `timeout` 单位均为秒，分别限制单次模型请求和整次执行。`--max-rounds`、`--max-tokens` 等可按任务覆盖；`--json` 约束答案，`--format json` 则包装 CLI 结果。
- **项目定制**：在 `general.sections` 中补充项目约定，或用 `prompt.sections` 覆盖某一节；子目录配置继承父目录，显式 CLI 参数优先。只有需要预设裸命令任务时才添加 `general.default_user`。
- **恢复**：中断或可恢复暂停后使用 `agent_tool xllm --resume --run <runid>`；恢复沿用已保存的配置、文件访问策略和提示词。修改模板后以新任务验证，已有 Run 的策略不会因修改 `.llm_context` 而改变，终态 Run 不会重新执行。

若使用 OpenAI 兼容服务，将模板的 `provider` 和 `model` 替换为以下配置，并在运行环境中设置 `XLLM_API_KEY`。占位地址和模型名必须替换为实际值；其余配置保持不变。

```yaml
provider:
  type: openai
  base_url: "https://your-provider.example/v1"
  api_key_env: XLLM_API_KEY
model: "your-tool-capable-model"
```

## 5. 功能需求

### F01. 开箱可理解的命令入口

- 提供独立的 `xllm` 命令，安装后可在 shell 和脚本中直接调用。
- 常规执行完成或失败后退出，不进入持续对话，不要求额外的 xllm 常驻服务；模型访问复用已配置的环境。简单文字任务不为启动 Agent、加载其它任务历史或维护长期记忆增加前置步骤。
- `--help` 优先展示 `xllm "问题"`、`xllm --select <name>` 和两个 xllm 管道串联的最短用法，再给出图片、本地任务、目录配置、任务列表和恢复示例；不需要模型连接即可查看。
- 配置好模型访问后，首次文字问答无需 `--user` 即可完成；首次图片问答也能用一条命令完成。常用组配置好默认任务后，`--select` 即可启动，不再要求重复输入问题。
- 无参数且没有管道输入时，若目录配置了默认组和非空默认任务，直接创建 Run 执行；未配置默认任务时显示简短用法，配置的组不存在时明确报错。有管道输入时先按 F02 接收并校验；空管道不能回退为执行默认任务。不挂起等待不明确的终端输入，不发起空模型请求。
- 模型服务未配置、认证失败或不可达时，说明用户需要修复什么，不能只展示内部堆栈。

### F02. 文本、图片与结构化单次输入

- 支持一句话、system 指令、文本文件、stdin，以及本地图片和图片 URL。`xllm "问题"` 的位置参数与 `--user <text>` 表达同一种任务要求，二者互斥；含空格的问题用 shell 引号作为一个参数传入。
- 图片首版支持 PNG、JPEG、WebP；实际模型限制更小时，应说明具体格式或尺寸限制。
- 支持重复 `--file` 和 `--image`，保留附件顺序和文件名；文件路径含空格、中文时正常工作。
- 常规输入组成一次 user 提交，显式材料按命令中的顺序组织，自动读取的 stdin 放在最后；`--system` 作为完整自定义业务指令，覆盖文件中配置的提示词，不与标准 section 重复拼接，也不移除 F11 定义的运行时协议提示词。
- 位置问题、`--user`、`--system` 和 `--select` 各接受一次；重复指定时报参数错误，不静默覆盖。`list`、`status`、`result` 保留子命令语义；要把同名文字作为问题时使用 `--user`。
- 本次任务要求优先使用显式位置问题或 `--user`，其次使用所选组的默认任务要求；显式要求替换组默认要求，不重复拼接。没有上述要求时，非空 stdin 本身作为 user 输入；否则 stdin 作为该任务的补充材料。仅有附件而没有任务要求时，提示补充要求。
- 有管道输入时自动读至 EOF，再开始模型与工具执行；没有管道时不自动等待终端 stdin。检测到管道但内容为空或仅有空白时，非零退出且不启动任务，即使指定了问题或组也不继续，以免上游失败后无材料执行。
- `--file` 表示把文本文件当材料；高级 `--input-file` 表示提交 SDK 定义的结构化单次任务输入，用于明确本次指令和材料。两者都创建新任务，不接续旧会话；已有对话内容若需分析，应作为本次材料显式提交。
- `--input-file` 与常规消息组装参数（位置问题、`--user`、`--system`、`--select`、`--file`、`--image`）互斥，也不自动合并管道内容或配置中的业务提示词、默认任务要求。业务输入只取文件中明确提交的本次内容，不加载已有 Run 的消息历史；Provider、模型和 loop 等执行配置仍按优先级生效，工具取顶层 tools、prompt.tools 和 CLI，不取未选组的 tools；运行时必需的协议提示词仍由系统注入。
- `--dir`、`--runs-dir`、附件路径、输入文件和 `--output` 的相对路径均基于启动命令时的 cwd；模型工具内部的相对路径基于工作目录。`.llm_context` 内的相对路径基于声明该值的配置文件所在目录。
- 文件不存在、不可读、类型不支持或输入超限时，指出具体材料和可采取的动作；不能丢弃附件后继续回答。

### F03. AI Provider 与模型选择

- `provider` 是接入配置对象，`provider.type` 首版支持 `buckyos`（默认）和 `openai`。buckyos 模式使用 AICC、taskmgr 等所需 BuckyOS 服务，可沿用当前身份或手工配置 `session_token`；openai 模式使用该接入方式的连接设置与凭据，不依赖 BuckyOS 服务。选择不同 Provider 不改变 CLI/SDK 的 Run 语义，也不要求启动 Jarvis 会话。
- `model` 是执行主任务的模型，可填写具体模型名或 Provider 支持的逻辑模型名，例如 `llm.chat`。`file_model` 是可选的附件理解模型，例如 `llm.vision`：首版用于处理图片，文本文件仍直接作为材料输入；配置它不代表首版自动支持 PDF、音视频。
- 配置 file_model 时，系统将真实图片、图片顺序、材料标签和本次任务要求提交给它，将分析结果连同来源交给主模型继续处理。多图比较必须保留图片之间的关联，不能对每张图孤立总结后丢失比较信息。文件模型调用属于显式输入准备，不受工具开关控制；关闭工具时仍可能有附件理解调用，但主任务只有一次推理，二者分别记录模型与用量。未配置 file_model 时，由主模型直接接收图片并检查其视觉能力；配置的文件模型不支持图片或调用失败时明确报错，不静默跳过或换模型。
- `--provider`、`--model`、`--file-model` 分别覆盖接入类型、主模型和文件模型；`--model` 不隐式覆盖 file_model。合并后的 Provider、模型能力和所选 loop 必须一起校验，配置不匹配时明确报错，不静默切换服务商或模型。主模型是否需要视觉能力取决于是否直接接收图片，不能在已经配置有效 file_model 时一概拒绝纯文本主模型。
- 启用工具、要求 JSON 输出等能力也参与模型可用性判断；不能为成功运行而静默去掉图片、工具或输出约束。
- 用户能从任务结果或状态中知道所用 Provider、主模型和文件模型、各阶段请求及实际返回的模型信息；无法取得的信息显示未知，不猜测。原始附件及已完成的文件模型分析随 Run 保存，恢复不重复已经确认完成的附件分析。
- 不在每次命令中重复要求用户输入凭据；支持复用已配置的 SDK 模型访问环境。首版提供 Provider 连接配置与选择，不承担账号管理、计费管理或本地模型安装。
- Provider 调用错误要区分可恢复与不可恢复：网络中断、单次请求超时、限流、临时服务故障等应保留可恢复进度；凭据过期等可通过修复同一凭据引用解决的问题，也允许修复后 resume。明确不支持的模型能力或无效请求不能靠无限重试掩盖，具体错误分类由 SDK/CLI 协议固定。

### F04. 工具、actions 与执行能力

- 工具是广义提示词配置的一部分。顶层 `tools` 提供基础配置，选中组的 `tools` 可覆盖它，`prompt.tools` 提供对所选组的目录级覆盖；有效工具配置按“产品默认 → 合并后的顶层 tools → 选中组的 tools → 合并后的 prompt.tools → 显式 CLI 开关”计算。对象按字段合并，`tools`、`actions` 和 `bash_tools` 列表分别整体替换，空列表清除对应继承项；未选组的工具不参与计算。
- 未配置 `enabled` 时默认关闭模型主动工具调用。组内 `tools.enabled: true` 即为显式启用，选择该组或执行目录默认任务时随之生效；CLI 的 `--tools` / `--no-tools` 覆盖所有文件配置。不开工具仍可读取用户通过 `--file`、`--image` 显式提交的材料。
- `tools.enabled: false` 或 `--no-tools` 同时关闭原生 tools 和 behavior actions，不连接 MCP 来发现或调用工具，主任务只进行一次推理。模型若仍请求工具或后续动作，应明确失败，不执行动作或自动追加修复推理；Provider 可恢复错误的有限重试与 resume 仍按 F05、F08 处理。
- 关闭工具时，仅处理本次输入与生效配置，不由模型自行扫描项目、执行命令或修改业务文件；本地写入仅包括 Runs 记录和用户显式指定的输出。常规执行不自动写回 `.llm_context`、创建 Agent 工作区或更新跨任务记忆。
- 启用工具后，支持读取文件、文本写入与编辑、执行本地命令，完成“分析—操作—检查结果”的多步任务。
- 本地命令执行继承启动 xllm 的当前 Bash 的 `PATH`，默认工作目录使用其 `CWD`；显式 `--dir` 可覆盖工作目录。
- 工具操作使用当前执行账户的权限。`--dir` 定义默认工作位置，不应被描述成已经提供操作系统级隔离。
- `tools.filesystem_policy` 支持 `workspace` 和 `unrestricted`，可随顶层、组内和 `prompt.tools` 的工具配置按字段覆盖。省略时为 `workspace`，限制内置文件工具路径及 exec 的 cwd；通用默认模板显式选择 `unrestricted`，允许跨工作目录读写与选择命令工作目录，由操作系统执行当前用户权限检查。该配置不改变 MCP/宿主工具权限，也不构成 shell 沙箱。策略随 Run 保存，恢复时沿用保存值。
- 用户可以提供自己的脚本或已安装命令；工具不可用时给出可读错误，由模型纠正或结束任务。
- 工具配置对象内的 `tools` 列表支持三种来源：`groupname` 引入内置工具组（如 `bash` 包含 `read_file`、`write_file`、`exec`），`mcp` 引入指定 MCP 服务提供的一组工具，`name` 选择单个已注册内置工具（如 `sendmsg`）。仅显式启用但未配置列表时使用默认 `bash` 组；显式空列表则不提供原生工具。未知组名、工具名、MCP 连接或工具发现失败应指出具体来源，不静默忽略；展开后的名称冲突必须报错或使用明确限定名消除歧义。
- `loop_model: function_call` 使用原生工具调用，默认不转换工具。`loop_model: behavior` 支持 `tools` 与 `actions` 两类入口；`tools2actions: true` 将生效的 tools 转为同等能力的 actions，原生 tools 列表置空，再与显式配置的 actions 合并并检查冲突。转换只改变调用协议，不改变参数含义、执行权限和限制。`tools2actions` 默认 false；function_call 模式下配置 actions 或启用 tools2actions 时报配置不匹配，不静默切换 loop。
- behavior 模式允许同时配置原生 tools 与 actions，但需要模型和执行器同时支持两种入口；相同操作不能因两种入口重复执行。两类操作共用工具开关、轮数限制、错误处理和执行记录。
- `bash_tools` 是 exec 的补充命令手册，每项至少包括名称、用途、调用命令和必要参数说明；用户不必将脚本改造成专用 API。它不替代结构化 `tools` / `actions` 列表，不会单独启用 exec，也不承诺构成 shell 命令白名单；exec 未启用时不将这些命令描述为可执行能力。
- 实际执行能力必须与提供给模型的工具说明一致，不能仅在提示词中宣称存在某项工具。
- 任务结束时列出已知生成的产物及其位置；对于未完成的文件或步骤，不描述为已经完成。
- 多步操作可留下已经产生的文件；失败或恢复不会自动回滚工作目录。状态和结果需要让用户看出哪些工作已有产物。

### F05. 独立任务的持久化与恢复

- 每次任务在生效的 Runs 目录中按 `runid` 保存记录，至少保留原工作目录、用户输入、使用过的附件材料、有效执行配置及其来源、已保存进度、最终结果与任务状态。配置快照包含 Provider、主模型与文件模型、loop_model、外部组展开后的内容、模板渲染后的提示词、最终 tools/actions、结果提取方式与限制；同时保留模型最终原始响应和提取结果。凭据仅保存来源引用，手工填写的 session_token 保存其原配置文件与字段位置，不复制到任务快照或日志中。
- 在首次 Provider 请求之前保存任务输入与初始进度，确保第一次模型请求就失败时也有可恢复的 runid；参数或材料预检失败、尚未建立 Run 的错误则明确说明没有可恢复任务。
- 保存后的输入应可用于恢复同一次任务；原始图片被移动或修改后，不能悄悄用另一张图片替换原任务材料。
- 普通调用总是新 Run、新 context，按本次 CLI、目录配置和默认值独立确定执行配置与额度。无论工作目录中已有任务是什么状态，都不从旧 Run 继承问题、回复、工具记录、摘要或执行配置；正在执行的任务仍受目录并发规则约束。
- Runs 记录用于查询结果和恢复原任务，不作为新任务的隐式输入。需要处理已有结果时，用文件或 stdin 显式传入所需材料；新任务只接收这些材料，不沿用原任务的完整上下文。
- `--resume` 默认选择当前 Runs 目录中属于当前工作目录的最近未完成任务；`--resume --run <id>` 只选择指定编号。已中断或因可恢复错误暂停的任务，在保存进度有效时继续，保留同一 `runid`；不存在目标或不可恢复时解释原因，不从头重做并称其为恢复。
- 可恢复错误保存最近错误、发生阶段及重试所需进度，本次命令非零退出，任务进入“暂停（可恢复错误）”，不写成终态失败。resume 重新尝试未完成步骤；再次出现可恢复错误时仍保存进度并退出，后续可继续 resume，直到成功或到达真正的终态。
- 已正常完成、因不可恢复错误失败或达到限制的任务为终态；对其调用 resume 只展示已保存的最终结果（包括失败或限制原因），不调用模型、执行工具、生成新 Run 或把状态改回执行中。终态结果缺失或损坏时报读取错误，也不能重新执行。
- 恢复沿用原工作目录、输入和有效执行配置，不因当前 cwd 或 `.llm_context`、外部组文件、环境变量变化而改变任务；非终态任务仍可通过显式 CLI 参数调整执行限制。要更换问题、附件、Provider、模型、loop_model、提示词或工具，应开始新任务。记录定位、日志级别与结果展示选项可以改变；调整执行限制也不能使终态任务重新运行。
- 恢复时重新连接原 Provider，并重新解析原凭据引用，使服务恢复或凭据修复能够生效；这不等于重新应用当前目录的模型、提示词或任务配置。已确认完成的模型响应和工具步骤保留，不因 Provider 错误从任务起点重做。
- 恢复前说明恢复的是哪个任务、保存进度的时间及接下来的阶段。已确认完成的工作应避免重做；无法确认的外部操作不能承诺绝不重复。
- resume 不接受位置问题、`--user`、`--system`、`--input-file`、`--select`、附件或管道输入，只恢复原任务目标与进度。普通新任务不接受 `--run`，不能用它指定或覆盖已有任务。
- 不同 Run 在关闭工具时可以从同一工作目录并行执行，记录按 runid 隔离；调用方负责避免显式输出文件重名。同一个 Run 的执行或恢复必须互斥，不能被多个进程重复推进。
- 启用工具的任务在同一工作目录内互斥，更换 Runs 目录也不能绕过该限制；冲突时快速报错并指出活动任务。关闭工具的独立任务不占用该工具执行锁；不同工作目录可独立运行，纯查询不占用执行锁。
- 同目录下的 `xllm ... | xllm ...` 必须可用：下游等待 stdin 时不占用执行锁，上游保存结果并释放其执行锁后再交付 stdout；不能因管道进程同时启动就误报目录并发或相互等待。

### F06. 任务状态与结果查看

`xllm list` 从当前生效的 Runs 目录中，展示属于当前工作目录的最近多次任务，按最近更新时间倒序排列，默认最多显示 20 条，可用 `--limit` 调整。每条至少包含 `runid`、任务概要、工作目录、状态、开始/更新时间及是否可恢复。先 `cd` 到项目目录再运行 `xllm list` 即可查看该目录的任务；显式 `--dir` 仅覆盖工作目录，不改变列表的筛选规则。没有记录时显示空列表并成功退出。

`xllm status` 至少展示：工作目录、Runs 目录、任务编号、任务概要、用户可理解的状态、开始/更新时间、有效执行配置及其来源，以及是否可以恢复。默认选择当前工作目录最新任务；指定 `--run` 时可直接查看该编号，不受当前工作目录过滤。

使用者可见状态至少区分：执行中、已完成、失败（不可恢复）、达到限制、已中断、暂停（可恢复错误），并明确是否为终态。可恢复错误展示最近一次失败原因、发生阶段、恢复条件及 resume 命令；不能只写“失败”让用户误以为必须重做。进程退出后残留的执行中记录不能永久冒充仍在运行，应能识别为中断或异常退出，并说明保存进度是否可恢复。终态不可恢复；执行中的任务不能被另一个进程重复恢复。

`xllm result` 读取已经保存的结果，不调用模型、不重复执行工具；默认选择当前工作目录最新任务，也可以指定 `--run`。默认沿用保存的 result_format，也可显式更换提取路径或改用 raw，从已保存的最终原始响应重新导出。任务尚未产生最终响应时，报告当前状态，不输出伪造的完整答案；重新导出不改变原任务状态。

list/status/result 和 `--resume` 均从当前生效的 Runs 目录查找记录，可通过 `--runs-dir` 覆盖；不隐式遍历其它位置。找不到指定编号时，显示本次查找的 Runs 路径。纯查询和终态 resume 不要求模型服务或原工作目录可用。

查询命令成功读取并交付记录时退出 0，记录中的任务状态另行表达；终态 resume 同样遵循查询语义。找不到任务、无法读取或输出保存失败时非零退出。用户查询一个失败任务，不等于查询命令本身执行失败。

### F07. 清晰的输出与脚本约定

| 选择 | stdout / 输出文件中的内容 |
| --- | --- |
| `result_format: raw`（默认） | 模型最终完成响应的原始内容；可以是文本、JSON 或 XML，不自动去除其结构 |
| `result_format: result.report` | 从最终 JSON/XML 响应中提取 report 字段或元素作为结果 |
| `--json` | 按 result_format 选出的结果必须是合法 JSON |
| `--format json` | 一个完整的结构化执行结果，包含任务状态及最终答案 |
| `--json --format json` | 结构化执行结果中包含 JSON 类型的答案，不把答案再编码成 JSON 字符串 |

- 结构化结果至少能表达任务编号、状态、是否终态、是否可恢复、答案、已知产物、可用的用量统计和本次调用的错误信息；字段协议另行定义。可恢复错误不能被编码成“任务已永久失败”。
- `--json` 是成功输出的约束。无法生成合法 JSON 时应明确失败，不能以成功状态返回一段非 JSON 文本。
- SDK 先按所选 loop 的执行协议识别动作、继续执行和任务完成，再对最终完成响应应用 `result_format`。raw 保留模型最终返回内容，包括其 JSON/XML 包装；它不包含 Provider 传输封装、整个 Run 历史或此前工具调用。只有完成响应可以交付，待执行动作和中途响应不能作为最终结果。
- `result.xxx` 中的 `result` 表示解析后的最终响应根节点，不要求 JSON 内再套一层同名字段；例如 `result.report` 从 `{"report":"评审结论"}` 或 XML 根元素的唯一 `<report>` 子元素提取文本。支持按点分隔的字段逐级访问；缺少字段、XML 路径匹配多个元素、结构解析失败或不支持的路径均明确报提取失败，不静默回退 raw。字符串按正文输出，JSON 对象/数组保留结构化值并在文本输出时序列化为 JSON；XML 文本元素输出文本，包含子元素的结果保留 XML 结构。
- 输出顺序为“确认任务完成 → result_format 提取 → --json 校验 → --format 包装 → stdout 或 --output”。`--json` 约束选出的结果，不要求 behavior 的全部协议响应都是 JSON；若 raw 保留 XML，则不能同时作为 JSON 成功输出。可预见的配置冲突在请求前报错，其余校验失败保留最终原始响应以便通过 result 重新导出，不自动增加推理修复格式。
- `result_format` 决定如何读取响应，`output_format` section 决定期望模型生成的内容，二者须对应；例如选择 result.report 时，应在提示词或运行时协议中约定 report 的承载。`--format` 只决定 CLI 的结果包装，重定向到文件或管道不会改变上述规则。
- `--format json` 的结构化 CLI 输出是一个可直接解析的 JSON 文档，不夹带日志、进度文字、颜色控制符或 Markdown 代码围栏；raw 模式保留的模型 XML 不受此包装规则改写。
- stderr 承担进度和诊断信息。stdout 被重定向或接入管道时，输出规则保持一致。
- 默认 CLI 文本模式只将按 result_format 选出的成功完成结果交付 stdout，不输出进度、错误说明或中途片段；这些诊断写入 stderr。下游按普通 stdin 消费结果，保持中文、换行与 JSON/XML 内容，不自动解释为消息历史或恢复指令。终态查询展示结果时仍遵循 F06 的查询语义。
- 每个管道阶段有独立的输入、执行配置、runid 与退出状态；上游配置或工具权限不通过答案自动传给下游。下游等待 EOF 后才执行，空管道按 F02 报错。普通 stdin 不携带上游退出码，脚本需自行检查管道各阶段状态，不能把最后一个命令成功等同于整个流程成功。
- 新任务和实际继续执行的 resume 只有任务正常完成且所要求的结果成功交付时才退出 0；输入错误、可恢复错误、本次任务不可恢复失败、达到限制、用户中断、输出写入失败均非零。非零退出不单独决定任务是否终止，应结合保存状态判断。只展示终态结果的 resume 按 F06 的查询语义退出。具体非零码由 CLI 协议固定并写入帮助文档。
- 在已经识别 `--format json` 的执行中，业务失败也输出可解析的错误结果；参数解析之前的错误至少给出明确 stderr 和非零退出码。
- `--output` 指向一个用户选择的最终输出文件，默认替换已有内容；不能在任务未完成时把半份答案当作成功结果覆盖进去。
- 输出目标的明显错误应尽早发现。模型已完成但输出保存失败时，分别说明“任务已完成”和“指定文件保存失败”，保留内部结果以便再次导出。

### F08. 运行限制与用量

- 用户可限制单次模型输出长度、工具轮数、单次 LLM 请求等待时长和本次命令的总执行时长。
- `--max-tokens` 控制单次模型输出上限，不冒充总 token 预算；`--max-rounds` 控制工具轮数，不冒充总推理次数。
- 首版工具轮数默认上限为 8，本次命令执行时长默认上限为 10 分钟；实际生效值可查看，用户可以覆盖。单次输出长度未设置时遵循模型配置。
- `max_tokens` 仍表示单次输出上限；示例的 256000 只有实际模型支持时才有效，不能被解释为上下文窗口或总 token 预算。`llm_timeout` 表示每次 LLM 请求超时秒数，默认 600，可通过 `--llm-timeout` 覆盖；它与 `--timeout` 的总执行时长独立，先触发的边界决定本次停止原因，总时长已耗尽时按达到限制处理。工具轮数对原生 tools 和 behavior actions 统一计数。
- 因执行限制耗尽而结束任务时，保存“达到限制”的终态结果，说明哪项限制生效并保留已有产物；此后 resume 只展示结果。需要调高限制继续工作时，应基于已有材料显式发起新任务，不重启原 `runid`。
- 对尚未到达终态的中断或可恢复错误暂停任务，恢复默认沿用原限制和已消耗的工具轮数额度；只有显式调高上限才增加可用轮数。本次命令执行时长从恢复启动重新计算，停机时间不消耗这次等待额度；不得借恢复重置已消耗的工具轮数。
- 单次 Provider 请求超时属于可恢复错误，与整个命令达到 `--timeout` 执行上限区分。若调用内部进行自动重试，必须有次数和时长边界；重试耗尽后以可恢复错误退出，不能无限等待，也不能把自动重试次数耗尽当成任务永久失败。
- 同一 Run 的工具循环积累的上下文接近模型容量时，可整理该任务的上下文继续执行；整理必须保留当前目标、关键结论、待办和必要的图片材料，并记录在任务状态中。这仅处理单次任务内部的容量，不跨 Run 累积或压缩会话历史。关闭工具的一次推理任务若输入超限，应明确报错，不自动追加模型压缩调用。
- 无法在模型容量内继续时明确停止；不无限重复压缩，不静默丢弃关键图片或当前任务要求。
- 服务提供 token 用量时展示实际统计；不知道费用时不显示虚构金额。文件模型、主模型及上下文整理调用分别记录并汇总可获得的用量，不能将关闭工具的任务描述为一定只有一次 Provider 请求。

### F09. 日志、进度、中断与可恢复错误

- CLI 自身的帮助、状态标签、进度日志和诊断信息统一使用英文；用户输入、模型结果和外部工具返回内容按原文保留。
- `run_logs` 支持 `debug`、`info`、`warn`、`result`，默认 info；debug 提供详细调试记录，info 显示阶段和工具进度，warn 仅显示警告与错误，result 隐藏常规过程日志，仅交付结果并保留失败/中断所必需的诊断。级别只改变输出详略，不改变执行行为、退出状态或持久化记录；凭据不随 debug 日志展开。
- info/debug 下，等待过程中至少能区分准备输入、等待模型、执行工具、整理上下文和保存结果；warn/result 下允许安静等待。
- info/debug 下显示实际阶段及耗时，不编造完成百分比；启动时显示合并了多少个路径上的 `.llm_context`，并按祖先到工作目录的顺序列出实际路径，未找到时显示 0。恢复时明确显示沿用原 Run 的配置来源。工具开始与完成时给出简短信息；exec 的括号中显示实际命令，失败时也保留命令，多行命令转义为单行显示，调用 ID 留在内部记录中。
- 用户按 Ctrl-C 后停止发起后续步骤，尽可能保存可恢复进度，并给出 `runid`、Runs 目录、保存是否成功，以及对应的 `xllm --resume --run <id>` 命令。尚未到达终态时记录为已中断；已保存的终态不能因 Ctrl-C 被改回可恢复状态。
- 可恢复错误同样停止推进并保存进度，说明本次命令已失败、任务仍可恢复，以及需要等待服务恢复还是修复凭据等条件；多次 resume 失败不会丢失原输入和已完成工作。
- 本地停止不等于服务端一定停止生成；不能把未确认取消的远端任务显示为已取消成功。
- 管道或脚本模式无需人工回答交互问题。遇到缺少必要信息的情况，以明确错误退出。

### F10. `.llm_context` 查找、合并与优先级

- 默认以当前 cwd 为工作目录，显式 `--dir` 可覆盖；再从工作目录逐级向上查找 `.llm_context`，直到文件系统根目录。不能遇到第一份文件或仓库根目录就停止。目录中没有配置文件是正常情况。
- 找到的文件按祖先到子目录的顺序合并。同名标量由更近的配置覆盖；对象按字段合并；列表整体替换，不隐式追加。未声明字段继续继承；允许空值的 section 和工具列表可显式清空，不能把空值当作“未配置”。
- 提示词组按组名合并；组内和目录级的 system section 均先将固定别名解析为行号，再按行号合并。同一行号的用户文本由子目录整段替换，不自动与父目录文本拼接；新增行号保留为独立 section，其余行号继续继承。section 集合按此规则逐项合并，不按普通列表整体替换。组内默认任务要求按标量覆盖。多个组的存在不代表全部启用，当前任务只选择一个组；显式 `--select` 优先于目录中的默认组选择。
- `prompt.groups.<name>` 可为内联组对象，或指向外部 YAML 的文件路径。外部文件根节点是单个组对象，可包含 default_user、sections 和 tools，不是整份 `.llm_context`。先按声明位置解析路径并展开组，再参与同名组的逐层合并；子目录可用内联差异覆盖父目录引用的组。外部文件中的相对路径以该文件目录为基准，不能因调用时 cwd 改变含义。
- 外部文件不存在、不可读、不是合法组对象或出现不支持的递归引用时，在模型请求前指出引用文件、组名和原因。Run 保存实际采用的组内容及来源；resume 不重新加载外部提示词文件。
- 组内 tools 按 F04 与顶层基础配置、prompt.tools 组合；只在最终选组后计算。Provider 类型切换时不继承另一类型的专属凭据和连接字段；无法形成有效配置时报错，不猜测字段含义。
- 合并后再应用显式 CLI 参数；没有传入的 CLI 选项不能用其默认值覆盖文件配置。Provider、模型、提示词和 Runs 目录等有对应 CLI 选项的设置均遵循此规则；任务恢复按 F05 使用已保存配置，不重新套用文件中的执行配置。
- 文件内路径相对于声明该值的 `.llm_context` 所在目录解析，继承到子目录后不改变含义；支持绝对路径及 `~` 表示当前用户主目录。Runs 目录没有文件或 CLI 配置时使用 `~/.xllm/runs`，新任务按需创建。
- 文件存在但不可读、格式错误、字段不支持、引用的提示词组不存在或合并结果无效时，应在模型请求前报错，指出文件、字段及原因，不能静默跳过。文件语法与字段类型由后续配置协议固定。
- 用户可从任务状态查看参与合并的配置文件、有效设置及主要覆盖来源；凭据等敏感内容不在输出中展开。

### F11. 提示词构造：system section、分组与完整自定义

#### F11.1 section 行号、固定别名与自定义插入

`.llm_context` 支持配置 system 提示词的 section。标准模式下，每个 section 都有一个正整数行号，作为其合并标识和排序位置，最终按行号从小到大拼接。行号是逻辑位置，不是配置文件或提示词正文的物理行数；一个 section 可以包含多行文本。系统为以下行号提供固定别名，用户可用行号或对应别名定位同一个 section；`.llm_context` 的具体文件语法由后续配置协议落实。

| 行号 | 固定别名 | 内容要求 | 示例 |
| --- | --- | --- | --- |
| 10 | `role` | 角色、职责和可复用的总体目标；本次具体任务放在 user 输入中 | “你是代码评审助手，重点发现影响正确性的问题。” |
| 20 | `contexts` / `env` | 当前时间、时区、操作系统、工作目录等运行环境，以及项目背景、领域知识和术语；两个别名指向同一 section | “当前时间为本次 Run 创建时的时间，工作目录为本次生效目录；这是一个 Rust 服务。” |
| 30 | `rules` | 执行原则、工作步骤和行为约束，必须说明本次哪些工具可以使用 | “本次已启用的工具为……；先读取相关实现，再提出修改，修改后验证。” |
| 40 | `cmd_manual`（cmd manual） | 可通过 exec 调用的命令手册，包括可用命令的名称、用途、调用方式和必要参数说明 | “项目检查命令：`./scripts/check.sh <target>`，用于验证指定模块。” |
| 100 | `output_format` | 最终答案的语言、组织方式、内容要求和格式偏好；不定义执行循环的内部输出协议 | “用中文回答，按问题、依据、建议组织结果。” |

- **任意位置插入**：用户可在提示词组或目录配置中指定尚未占用的行号，新增自己的 section，并可附带名称或标题。例如 25 位于 `contexts` / `env` 与 `rules` 之间，50 位于 `cmd_manual` 与 `output_format` 之间；也可用 5 或 110 插入到这些预设 section 之前或之后，无需重编号其它 section。100 是 `output_format` 的固定位置，不是行号上限。
- **按行号定位与覆盖**：固定别名不能改绑到其它行号；例如 `rules` 与 30 等价，`contexts`、`env` 与 20 等价。自定义 section 的名称或标题不决定顺序，覆盖已有行号表示替换该 section 的用户文本，插入新 section 应使用不同的行号。
- **冲突可诊断**：同一配置层的同一 section 集合中，若多个声明解析到同一行号（包括同时使用数字和别名，或同时使用 `contexts` 与 `env`），应报配置冲突，不按书写顺序任选一个。非法行号、固定别名与行号不匹配、仅引用未知别名却未给行号时同样报错；带有效行号的自定义 section 不属于未知字段错误。

这些 section 都属于可复用的 system 指令。组内默认任务要求、本次问题、文本附件、图片和 stdin 属于 user 输入，不是额外的 system section；模型回复和工具结果只在本次 Run 执行过程中产生，也不属于可配置 section。

系统在新 Run 构造提示词时为 `contexts` / `env` 提供本次时间和实际运行环境，用户可通过 F11.4 的模板变量安排这些信息的位置并补充项目背景；已由模板呈现的信息不重复追加。`rules` 根据最终展开和转换后的 tools/actions 说明实际可用能力，`cmd_manual` 根据 exec 是否启用及 bash_tools 提供命令手册。用户可补充使用规则、命令示例和注意事项，无需重复抄写系统已提供的说明。关闭工具时，`rules` 明确说明没有可调用工具或动作，不提供可执行的命令手册；未启用的能力不能被描述为可用。

系统生成的环境信息、可用工具说明和命令手册与用户可编辑文本分开管理，在合并用户文本后填入对应行号。覆盖或清空用户文本不移除系统依据生效配置生成的说明，也不改变实际工具开关；写入工具或命令名称不会安装或启用它。实际工具声明及参数约定仍由系统提供。`output_format` 同样不能取消 `--json` 等显式输出约束；`--format` 只控制 CLI 结果包装，不是答案提示词 section。

#### F11.2 标准 loop、分组与系统组装顺序

标准 loop 提供宽泛的执行框架，具体任务的角色、背景、方法和交付要求由上述业务 section 与本次输入决定。系统提示词只规定通用执行约定，不内置代码评审、修复 issue 等特定业务流程，也不要求所有任务输出同样的计划、报告或固定步骤。用户设置的 `rules` 可以指导本次工作流程，`output_format` 可以决定最终答案的形式。

除可配置 section 外，系统保留 **`runtime_protocol` 通用循环与运行时协议部分**，至少说明：

- **如何推进任务**：根据目标和已有结果决定下一步；可以直接完成，也可以经过多轮执行。工具失败或结果不足时，可在执行限制内调整做法继续。
- **如何使用能力和观察结果**：按实际可用工具及参数约定发起操作，依据返回结果判断进展，区分计划执行、已经执行和已经确认完成。
- **如何表达当前状态与交付结果**：使执行器能够区分继续执行、待执行动作和任务完成，并识别最终答案；必要的步骤记录、结果反馈格式由执行协议规定。
- **如何遵守执行边界**：遵守工具开关和执行限制，遇到中断、可恢复错误或终止条件时按任务状态规则处理；提示词不能自行扩大实际执行权限。

`runtime_protocol` 由 SDK 按实际执行器、解析器和生效配置提供，不是 `.llm_context` 可配置字段，不占用可配置 section 的行号，也不参与组继承、目录覆盖或显式清空；它在所有按行号组装的 section 之后追加。与任务相关的工具偏好和业务步骤放在可配置的 `cmd_manual`、`rules` 或自定义 section 中，避免把某类任务的做法固化为所有任务的必需协议。

`loop_model` 决定协议装配：function_call 使用原生工具调用，behavior 使用与执行器匹配的行为协议。若采用 XML behavior loop，系统须补齐 `<response>`、`<actions>`、`<report>` 等实际使用结构的说明，并与解析器匹配。用户可选择 loop，但无需手写运行时控制说明；协议只保留当前执行环境支持的能力，不要求用户配置多个 behavior 或切换状态机。tools2actions 转换后的实际能力必须同时反映在规则、工具声明和协议说明中；`--no-tools` 关闭两种入口，并装配一次推理直接完成所需的说明。

- 父目录可以定义多个命名提示词组，例如 `review`、`docs`、`fixissue`；每个组提供一套带行号的 section 内容，可包含预设与用户新增的 section，也可附带独立于 system section 的默认任务要求。子目录继承组定义，选择其中一个组，并可提供本目录的 section 覆盖值或新增行号；更近目录声明的组选择覆盖父目录选择。
- `--select <name>` 从合并后的组定义中选择一个组，覆盖文件中的模式和默认组选择，并采用标准 section 组装。组内默认任务要求作为本次 user 要求使用，优先级低于位置问题或 `--user` 的显式要求；仅有角色、风格等 section 而没有任务要求的组，不能在无输入时启动空任务。

新任务默认采用标准模式，系统按以下步骤组装：

1. **合并配置并确定模式与组**：按 F10 展开外部组并从最远父目录到工作目录合并配置，再应用显式 CLI 参数，确定本次提示词模式、loop_model 和唯一选中的组；未选组时跳过组内容。按 F04 计算工具配置并完成 tools2actions 转换后，确定实际可用能力。
2. **计算每个 section 的最终文本**：将固定别名归一到行号，依次取“系统默认值 → 选中组的值 → 合并后的目录 section 覆盖值”，后层声明的同一行号整段替换前层的可配置文本，新行号作为独立 section 加入。未声明则继承，显式空文本则清空，不把多个配置来源的文本叠加。切换组时重新计算，不保留上一个组的文本。随后按 F11.4 渲染模板，按 F11.1 补齐实际环境、可用工具和 exec 命令说明。
3. **组装 system 提示词**：将最终非空 section 按行号升序各拼接一次，自定义 section 按其行号插入；再追加 `runtime_protocol` 中的通用执行规则与当前协议说明，保留可辨认的边界。最终内容为空的 section 连同标题一起省略；系统生成的必要说明、通用循环与必需协议不能因清空用户文本而省略。配置文件中的字段书写顺序不影响结果。这里的排列顺序与上一步的配置覆盖优先级是两回事，协议是否有效还必须由运行时解析和执行校验保证。
4. **组装一次 user 提交**：先放本次任务要求（显式位置问题或 `--user` 优先，其次为已渲染的组默认任务），再放按命令顺序组织的 `--file` / `--image` 材料，最后放自动读取的 stdin。启用 file_model 时按 F03 完成图片理解，主模型接收带原始标签和关联的分析结果，Run 仍保留原始图片。没有其它任务要求时，非空 stdin 自身作为任务要求，只放一次；空输入和空管道的处理遵循 F02。
5. **执行并组织本次记录**：主任务首次请求使用上述 system 和 user 内容，并按生效配置提供工具声明及输出约束；后续由所选执行循环追加或渲染本次 Run 的模型回复、步骤记录和工具结果，持续保留必需协议说明，不加载其它 Run 的历史。最终完成响应按 F07 提取、校验和包装后交付用户。

```text
system（标准，无新增 section）：10 role → 20 contexts/env → 30 rules → 40 cmd_manual → 100 output_format → runtime_protocol
system（插入 25、50 后）：10 role → 20 contexts/env → 25 项目约定 → 30 rules → 40 cmd_manual → 50 检查清单 → 100 output_format → runtime_protocol
system（自定义）：用户整段业务提示词 → runtime_protocol
user：本次任务要求 → 显式附件（命令顺序）→ stdin（若作为材料）
后续：本次 Run 的模型回复、步骤与工具结果 → 下一次模型调用……
```

其中最终内容为空的 section 跳过；`runtime_protocol` 始终由系统提供，具体结构化输出说明随实际协议确定，不因用户替换业务提示词而删除。

例如，`review` 组提供 `role`、`rules`、`output_format` 和 25 行的“项目约定”，当前目录用 100 行覆盖输出要求，再新增 50 行的“检查清单”，则其余 section 正确继承，并按 10、20、25、30、40、50、100 的顺序组装；组内原有 `output_format` 用户文本不再出现。行号 100 与别名 `output_format` 定位到同一 section，不产生重复内容。

#### F11.3 完整自定义与恢复

- 完整自定义模式直接采用用户配置的整段业务提示词，不再按行号拼接预设或用户新增的业务 section，也不注入组内容，仍追加必需的 `runtime_protocol`。子目录可显式切换模式；同一层同时声明完整自定义模式与组选择或 section 配置时报配置冲突，不能猜测拼接方式。仅定义供子目录复用的组不构成模式冲突。
- 沿用 `--system` 作为本次完整自定义提示词覆盖，优先于文件中的模式、组和 section 配置；与显式 `--select` 互斥，避免同时要求完整自定义与标准组装。
- 完整自定义模式和 `--system` 仅替换 system 中的业务部分；user 输入仍按 F02 组装，不从未启用的组取默认任务要求或工具配置，工具只取顶层 tools、prompt.tools 和 CLI 的有效值。实际工具声明和输出约束继续按生效配置提供；它们与必需的运行时协议均不受业务文本覆盖。`--input-file` 也遵循这一边界，不能绕过执行协议。
- 提示词内容不会改变实际工具开关、可用工具列表、模型能力或执行限制；完整自定义提示词也必须服从这些执行配置。`--input-file` 的提示词处理遵循 F02。
- 配置来源、外部组文件、选择的组、提示词模式与 loop_model、各 section 的行号、别名或名称、最终内容与来源、已使用的模板变量及渲染结果、运行时协议内容及版本、拼接后的提示词和实际任务要求应随任务保存，确保查看时能核对组装结果。resume 直接使用保存的提示词、输入与本次 Run 的进度，不重新执行目录配置合并、模板渲染或 section 组装，不因组定义修改而更换任务，也不重新填入时间或环境信息；当前执行器无法处理保存的协议版本时明确报错，不静默换用新协议。

#### F11.4 环境模板变量

- 配置中的 section.text、default_user 和完整自定义 system 支持模板替换。首版至少提供本次 Run 的时间、时区、操作系统、工作目录，以及显式按名称引用的进程环境变量；示例采用 `{{runtime.current_time}}`、`{{runtime.timezone}}`、`{{runtime.os}}`、`{{runtime.cwd}}` 和 `{{env.PROJECT_NAME}}`。这些命名与转义写法由配置协议统一，不要求用户硬编码每次调用的环境。
- 先完成配置合并与组选取，再只渲染最终生效的提示词文本；未选组、被覆盖文本不因缺少变量而影响本次任务。运行时内置变量由系统提供，环境变量仅按显式引用读取；不执行模板里的 shell 命令或代码，也不递归解释替换值中的模板。未知变量或所需环境变量缺失时，在模型调用前指出来源与变量名，不悄悄填空。
- 同一次 Run 的初始模板值和渲染结果固定并保存，resume 不重新取当前时间、cwd 或环境变量。用户提交的问题、附件和 stdin 作为材料原样处理，不自动当作配置模板；凭据字段也不混入提示词模板上下文。

## 6. 首版命令面

以下沿用现有命令风格，只固定产品层面的入口与含义。完整语法、字段 schema 和错误码由后续 CLI/SDK 协议细化；配置来源与优先级遵循 F10、F11。

| 入口/参数 | 使用者意图 | 首版默认 |
| --- | --- | --- |
| `xllm` | 执行目录默认任务 | 有默认组及默认任务时新建 Run；无有效任务时显示用法 |
| `xllm "问题"` | 直接给出任务要求 | 手工输入的首选方式，与 `--user` 等价且互斥 |
| `--user <text>` | 显式给出任务要求 | 保留现有写法 |
| `--select <name>` | 选择已有提示词组 | 覆盖目录默认选择；无显式问题时使用组默认任务要求 |
| `--system <text>` | 给出本次任务的完整自定义业务指令 | 覆盖文件业务提示词，保留必需的运行时协议；未设置时按配置或默认值组装 |
| `--file <path>` | 提交文本材料 | 可重复 |
| `--image <path-or-url>` | 提交图片 | 可重复 |
| stdin | 提交管道文本或上一个 xllm 的答案 | 有任务要求时作为材料，否则作为 user 输入；空管道报错 |
| `--input-file <path>` | 提交结构化单次任务输入 | 创建新任务，与常规输入组装互斥 |
| `--dir <path>` | 不切换 cwd，显式指定其它工作目录 | 默认使用当前目录，日常 cd 后直接调用即可 |
| `--runs-dir <path>` | 指定任务记录目录 | `.llm_context` 配置，否则 `~/.xllm/runs` |
| `--provider <type>` | 选择 Provider 接入方式 | `.llm_context` 中的 provider.type，否则 buckyos；首版也支持 openai |
| `--model <name>` | 选择主任务模型 | `.llm_context` 配置，否则使用环境默认值 |
| `--file-model <name>` | 选择附件理解模型 | `.llm_context` 配置；未配置时由主模型直接处理并校验能力 |
| `--loop-model function_call\|behavior` | 选择执行循环 | function_call；与文件中的工具配置共同校验 |
| `--tools` / `--no-tools` | 允许/关闭所有模型 tools/actions 调用 | 覆盖组和目录开关；未配置时默认关闭；两个参数互斥 |
| `--resume` | 继续中断或可恢复错误暂停的任务；终态只展示结果 | 当前工作目录最近未完成任务；可配合 `--run` 指定编号 |
| `--max-tokens <n>` | 限制单次模型输出长度 | 模型配置 |
| `--max-rounds <n>` | 限制工具轮数 | 8；0 表示不执行工具 |
| `--timeout <seconds>` | 限制本次命令等待时长 | 3600 秒 |
| `--llm-timeout <seconds>` | 限制单次 LLM 请求等待时长 | `.llm_context` 中的 llm_timeout，否则 600 秒 |
| `--run-logs debug\|info\|warn\|result` | 控制 stderr 过程日志详略 | `.llm_context` 中的 run_logs，否则 info |
| `--result-format raw\|result.<path>` | 保留最终原始响应或提取字段 | `.llm_context` 中的 result_format，否则 raw |
| `--json` | 要求答案为 JSON | 默认普通文本答案 |
| `--format text\|json` | 选择 CLI 结果格式 | text |
| `--output <path>` | 保存最终输出 | 默认 stdout |
| `xllm list` | 查看最近的多次任务及 runid | 当前工作目录的任务，从生效的 Runs 目录读取，按更新时间倒序 |
| list 的 `--limit <n>` | 指定最多显示的任务数 | 20 |
| `xllm status` | 查看任务是否完成、是否可恢复 | 当前目录最新任务 |
| `xllm result` | 读取已保存结果 | 当前目录最新任务；支持 result-format/format/output，重新提取不调用模型 |
| status/result/`--resume` 的 `--run <id>` | 查看或恢复指定任务 | 不用于给新任务指定编号或接续旧会话 |
| `--help` / `--version` | 了解用法和版本 | 不依赖模型连接 |

新任务的显式参数覆盖 `.llm_context` 和环境默认值，不读取上一次 Run 的配置或对话；resume 使用同一次任务保存的执行配置，仅允许为非终态任务显式调整执行限制。对终态任务传入执行限制时报参数不适用，不静默忽略，也不重启任务。任何接受但不生效的参数都属于缺陷。

## 7. 异常体验

| 情况 | 用户应该看到什么 | 可以怎样继续 |
| --- | --- | --- |
| 图片路径错误 | 哪张图片不存在，按哪个路径解析 | 修改路径后重试 |
| 图片太大或格式不支持 | 具体限制和受影响材料 | 转换/缩小图片，或换合适模型 |
| 图片理解模型能力不足 | file_model 或直接接收图片的主模型不支持图片，本次未丢弃材料执行 | 配置支持图片的文件模型或主模型 |
| Runs 目录不可写，或任务所需产物目录不可写 | 不能保存记录或产物的具体位置 | 更换对应目录或修复访问权限 |
| `.llm_context` 无效 | 出错文件、字段及原因，包括不存在的组、未知 section 别名、非法行号或同层行号冲突 | 修正配置后重试 |
| 外部组文件或模板无效 | 引用文件、组名、缺失变量或渲染错误；尚未请求模型 | 修正路径、组内容或设置所需环境变量 |
| 工具来源或 loop 不匹配 | 未知工具/组、MCP 连接失败、名称冲突或不支持的 tools/actions 组合 | 修正实际工具配置或选择兼容的 loop/模型 |
| result_format 提取失败 | 最终原始响应已保存，明确指出解析错误或缺失路径；本次结果交付非零退出 | 用 result 指定正确提取路径或 raw 重新导出，无需重新推理 |
| `--select` 的组不存在或无任务要求 | 组名、可用组或缺少的任务要求 | 选择已有组，补充位置问题或配置默认任务 |
| 管道输入为空 | 未收到可供执行的输入，本次未调用模型 | 检查上游结果与退出状态后重试 |
| 同目录已有启用工具的任务，本次也需启用工具 | 正在执行的任务编号与状态；本次未开始工具执行 | 等待原任务结束，或使用另一个工作目录；关闭工具的独立任务可并行 |
| Provider 网络错误、请求超时、限流或临时故障 | 本次命令非零退出，任务暂停且可恢复；错误、runid 和恢复命令 | 服务恢复后 resume；若再次失败，仍可继续恢复同一任务 |
| Provider 凭据过期或暂时不可用 | 可修复的认证问题，原任务进度已保留 | 修复原凭据引用后 resume，不重输问题 |
| 请求或能力不受支持等不可恢复错误 | 具体原因及任务是否已终止；预检失败时说明尚未建立 Run | 修正请求后创建新任务，不反复重试原无效请求 |
| 工具失败 | 失败的操作及它对任务的影响 | 任务内自动纠正；若任务已失败终止，则修复后发起新任务 |
| 达到工具/时间限制 | 已完成部分、触发的限制、已到达终态 | 调整限制后发起新任务；resume 仅展示原结果 |
| 没有可恢复记录 | 为什么不能执行指定操作 | 发起新任务或查看已有状态 |
| 找不到指定 runid | 本次查找的 Runs 目录和任务编号 | 用 list 核对编号，或指定正确的 `--runs-dir` |
| 对终态任务执行 resume | 已保存的最终结果与终止原因 | 查看结果；需要新工作时另建任务 |
| 保存记录损坏 | 哪次任务无法恢复，不伪装为重新开始 | 查看已有产物，显式发起新任务 |
| 输出保存失败 | 任务是否已经完成、已有结果如何取回 | 用 `xllm result` 重新导出 |
| 用户中断 | 是否保存了进度、后续操作命令 | 查看状态或恢复 |

## 8. 版本范围

### 8.1 首版必须完成

首版交付 F01–F11 中的用户体验，包括：位置参数直接提问、目录默认任务与显式选组、文字和图片输入、主模型与文件模型、管道串联、`.llm_context` 向上查找与合并、buckyos/openai Provider 接入、function_call/behavior 两种 loop、section 行号与固定别名、自定义插入、环境模板、内联与外部提示词组、完整自定义提示词、组内工具配置、内置组/MCP/具名工具、tools2actions、exec 命令手册、四档日志与结果字段提取、Runs 目录、独立任务与恢复、最近任务列表、状态与结果查询、结构化结果、执行限制和中断处理。可恢复错误只使本次命令失败，原任务可反复 resume；终态任务只读展示结果，不得重新执行。跨任务只通过用户显式选择的输入传递材料，不提供持续 session。

首版先支持已有 TS agent tools 本地执行环境适用的 Linux/macOS 命令行。Windows 原生工具执行另行验收，不因 Node.js 可运行就宣称完全支持。

### 8.2 后续能力

- PDF、音频、视频等更广泛的媒体材料，以及多种媒体的混合输入。
- 流式答案和机器可订阅的事件流。
- 任务列表的高级检索和记录管理；基础最近任务列表已在首版范围内。
- 用 JSON Schema 等方式约束模型结果的字段。
- 后台任务提交、长时间异步等待及跨进程回调。
- 批量任务、可分发的模板库和更细粒度的工具策略；目录提示词组、外部组文件和环境变量替换已在首版范围内。

### 8.3 独立处理的工作

- 修改 Rust `LocalLLMContext` 或迁移它现有的 OpenDAN/Jarvis 调用方。
- 兼容旧 `run_local_llm` 命令与快照。
- 持续会话、跨任务滚动历史、会话分支、长期记忆、由用户配置多个 behavior 及其切换状态机、多 Agent 协作；这些不属于 xllm 的 oneshot 定位。单个 Run 内通用 loop 的多轮执行属于首版能力。
- Provider 账号与服务管理、本地模型安装、完整计费平台；已有 Provider 的连接配置和选择在首版范围内。

这些工作不构成 xllm 首版发布的前置条件。首版仍需要可用的模型访问环境；buckyos Provider 依赖其所需的 AICC、taskmgr 等服务，openai Provider 不依赖这些 BuckyOS 服务。“无需启动 Jarvis”不等于无需 Provider 及所选工具依赖的服务。

### 8.4 与后续 Agent 命令行工具的关系

xllm 完成后的后续规划，是基于 OpenDAN Agent Session 提供面向单个 task 完成的命令行工具，复用 Agent 身份、配置及跨 session 的标准 Agent Memory、Notebook 等能力。一次任务完成后命令退出，Agent 的长期状态继续保留。xllm 继续保持 AICC 与 Agent 之间的独立执行边界，不要求日常小脚本承担 Agent 会话和长期记忆的管理。

## 9. 产品验收

验收以使用者可观察的结果为准，具体实现测试另行组织。

| 编号 | 场景 | 通过条件 |
| --- | --- | --- |
| A01 | 首次文字问答 | 配置好模型访问后，`xllm "问题"` 直接运行，与 `--user` 行为等价；不要求用户创建 Agent 或编辑内部 JSON |
| A02 | 本地图片＋一句话 | 无需手工 Base64/上传，模型能收到实际图片；回答与该图相关 |
| A03 | 两张图片比较 | 图片顺序保持正确；不会只发送其中一张或把文件名当成图片 |
| A04 | 文本文件和 stdin | 内容完整进入任务；文件名标签、中文和换行不被意外破坏 |
| A05 | 非法输入 | 文件不存在、空任务、非法参数等在能预检时先报错，不静默改变任务 |
| A06 | 工具开关优先级 | 未配置时模型不主动调用工具；所选组 enabled=true 可启用工具，prompt.tools 可覆盖组配置，--no-tools 覆盖全部文件开关并关闭 tools/actions；显式附件仍可读取 |
| A07 | 开启工具生成索引 | 本地命令使用当前 Bash 的 `PATH`，未指定 `--dir` 时在其 `CWD` 读取材料并生成索引，显式 `--dir` 可覆盖工作目录；报告真实产物路径 |
| A08 | 新任务上下文独立 | 同目录连续运行两次任务，第二次 context 不包含第一次的对话、工具记录、摘要或临时配置；只使用本次输入与生效配置，旧记录和业务文件仍可访问 |
| A09 | 中断与恢复 | Ctrl-C 后能看到 runid 和真实状态；`--resume` 或 `--resume --run <id>` 无需重输问题和图片即可继续非终态任务；无有效恢复进度时明确说明 |
| A10 | 新任务与恢复区分 | 重复普通命令创建新 Run 和干净 context；resume 只恢复原任务，不创建替代空任务，不接受新问题、附件或组选择 |
| A11 | JSON 答案 | `--json` 成功结果能被 JSON 解析器直接解析；无效 JSON 不以成功交付 |
| A12 | 结构化 CLI 输出 | stdout 是一个完整 JSON 结果；stderr 的进度不会污染它；失败状态可被脚本识别 |
| A13 | 结果保存与取回 | output 保存正确格式；写入失败可通过 result 取回已有结果，无需再次调用模型 |
| A14 | 执行限制 | 达到轮数或时间限制后停止推进并保存终态结果；原因和已完成部分可见；resume 不重新执行 |
| A15 | 单次任务上下文超过模型容量 | 只整理该 Run 内的上下文后继续，或给出明确容量错误；不加载其它 Run 历史、不无限循环、不悄悄丢关键图片 |
| A16 | 目录并发与管道 | 同目录的多个无工具 Run 可并行且记录互不覆盖；同一 Run 不能并发推进，同目录的多个工具任务不能并发执行；管道下游等待完整输入后可执行，不误报占用或死锁；不同目录任务独立运行 |
| A17 | 材料变化后的恢复 | 保存任务后移动或修改原图片，恢复使用原任务材料或明确失败，不静默换图 |
| A18 | 查询不触发执行 | list/status/result 只读取记录，不新增模型调用或工具副作用 |
| A19 | CLI 与 SDK 一致 | 相同请求、执行配置和依赖下，CLI 与直接 SDK 调用具有相同任务语义 |
| A20 | 无额外运行时门槛 | 在模型访问可用的前提下，无需运行 Jarvis 会话即可完成文字、图片和本地工具任务 |
| A21 | 配置向上查找与覆盖 | 工作目录没有文件时仍查找父目录；多层文件按祖先到子目录合并；显式 CLI 参数最高优先级，未传参数不覆盖文件配置 |
| A22 | Provider 与模型配置 | provider.type 默认 buckyos，也支持 openai；buckyos 可沿用身份或指定 session_token，openai 不依赖 BuckyOS 服务；主模型与文件模型分别生效，CLI 对应参数可覆盖；凭据不写入日志/快照；能力不匹配在请求前报错 |
| A23 | 提示词组与 section | 父目录定义多个组，子目录选组并按行号或固定别名覆盖一个 section；其它 section 正确继承，按行号升序各拼接一次，不受配置字段书写顺序影响；最终内容为空的 section 不输出标题或正文，不混入被覆盖或未选组内容；任务要求与附件位于其后的 user 输入中 |
| A24 | 完整自定义提示词 | 业务部分只使用整段内容；`--system` 优先于文件配置；不额外拼接预设或用户新增的业务 section，必需的 runtime_protocol 仍注入，实际工具与限制仍生效 |
| A25 | exec 命令手册 | exec 启用时模型能获知配置的 bash_tools 命令及参数；子目录列表替换和空列表清除生效；仅配置手册不启用 exec，未启用时不将命令描述为可执行能力 |
| A26 | Runs 目录 | 无配置时保存到 `~/.xllm/runs`，文件和 CLI 可覆盖；继承的相对路径保持声明目录语义，任务记录保留原工作目录 |
| A27 | 最近任务列表与编号定位 | cd 到项目后直接 list 展示该工作目录的最近任务；与显式指定同一 `--dir` 的结果一致；可按列出的 runid 查询或恢复，空列表和自定义 Runs 目录行为一致 |
| A28 | 终态不可重跑 | 对正常完成、不可恢复失败、达到限制的任务执行 `--resume --run <id>`，只展示原结果；不新增模型/工具调用、不新建 Run、不改变终态；结果损坏也不重跑 |
| A29 | 配置变化后的恢复 | 中断后修改目录配置或从其它 cwd 按编号恢复，仍使用原工作目录与保存的执行配置；显式调整限制不重置已消耗轮数，也不能重新执行终态任务 |
| A30 | 配置错误可诊断 | 非法文件、未知字段或 section 别名、非法行号、固定别名与行号不匹配、同一集合内同层行号重复、不存在的组及冲突模式在请求模型前报错；带有效行号的自定义 section 可正常使用；输出指出来源且不泄露凭据 |
| A31 | 短命令选择任务 | 配置 `fixissue` 组和默认任务后，`xllm --select fixissue` 无需再写问题即可启动；显式问题覆盖默认任务，CLI 选组覆盖目录选择，组不存在或任务为空时不发空请求 |
| A32 | xllm 管道串联 | `xllm "任务一" \| xllm "任务二"` 无需中间文件；下游以新 context 接收上游答案，不继承其对话和工具记录；两次任务各有 runid，日志不混入，JSON 答案也可作为材料传递 |
| A33 | stdin 输入与空管道 | 仅有非空 stdin 时作为 user 输入启动任务；有问题或组默认任务时作为补充材料；上游默认模式失败无 stdout，下游因空输入非零退出且不调用模型 |
| A34 | Provider 错误后恢复 | 首次或后续模型请求遇可恢复错误时，命令非零退出但任务非终态且保留进度；服务恢复后 resume 沿用 runid、原输入和已完成步骤并成功结束 |
| A35 | 多次恢复失败 | 同一 Run 多次 resume 遇到可恢复错误后仍可恢复，不因重试次数耗尽而永久失败；每次命令及时退出并给出错误，最终恢复不重复已确认完成的工具步骤 |
| A36 | 凭据修复与状态表达 | 修复原凭据引用后 resume 能重新连接；list/status/结构化结果清楚区分本次调用失败、可恢复暂停与真正终态失败 |
| A37 | 运行时协议与结果处理 | 标准、自定义和 input-file 模式均装配与 loop 匹配的协议；业务 section 覆盖不能删除协议；no-tools 关闭所有执行入口；只有最终完成响应可交付，raw 保留其结构，字段模式按路径提取后再校验 JSON；恢复保留原协议，不兼容时明确失败 |
| A38 | 通用执行框架 | 问答、总结、写作和工具任务可选择 function_call 或 behavior，二者遵循相同的任务、限制和恢复规则；简单任务直接完成，复杂任务可多轮操作；无需业务专用 loop 或多个 behavior 的切换状态机 |
| A39 | 小脚本中的副作用边界 | 同一脚本反复使用 `--no-tools` 处理材料并按退出状态分支，无需人工交互或 Agent 会话；模型不自行读取其它业务文件、执行命令或写入长期状态；本地仅新增 Runs 记录及显式输出，不自动创建项目工作区、修改目录配置或启动后续后台任务 |
| A40 | section 行号与插入 | 固定别名正确映射为 role=10、contexts/env=20、rules=30、cmd_manual=40、output_format=100；新增 5、25、50、110 行时出现在对应位置，无需改动预设行号；子目录可覆盖或清空继承的自定义 section，数字与别名跨层覆盖不重复输出；resume 保留原行号、内容和顺序 |
| A41 | 环境、工具与命令说明 | contexts/env 呈现本次时间与环境，rules 对应最终 tools/actions，cmd_manual 对应实际可用 exec 命令；清空业务文本不移除系统必要说明；转换工具后说明同步变化，关闭工具后不宣称能力可用 |
| A42 | 目录默认任务 | 默认组有 default_user 时直接运行 xllm 创建新 Run，重复调用彼此独立；无默认任务时显示用法；显式问题/选组可覆盖，空管道仍报错且不执行默认任务 |
| A43 | 外部提示词组 | groups 中的路径按声明目录解析，外部文件作为组对象参与合并；子目录可覆盖其 section/tools；无效文件可诊断；修改或删除源文件后 resume 仍使用已保存的展开内容 |
| A44 | 工具来源与组覆盖 | 所选组的工具可覆盖顶层基础配置，prompt.tools 再覆盖组，CLI 开关最高；内置组、MCP 和具名工具均能展开，未选组能力不混入；列表替换/清空有效，来源错误和名称冲突不静默跳过 |
| A45 | loop 与 tools2actions | 默认 function_call；behavior 下转换保持工具语义和参数，原生 tools 置空并合并显式 actions；支持兼容的 tools/actions 并存且不重复执行；关闭开关后主任务仅一次推理，不发现或执行 MCP 工具；不兼容组合在请求前报错 |
| A46 | 文件模型分工 | 有 file_model 时由它接收真实图片及任务要求，主模型接收带来源的分析；无附件时不调用它，未配置时由支持视觉的主模型直接处理；多图关联保留，调用与用量可区分；恢复不重复已完成的文件分析 |
| A47 | 模板替换与恢复 | 生效的 section/default_user/system 可引用 runtime 与具名 env 变量，渲染顺序在覆盖之后；缺失变量在请求前报错，被覆盖/未选组模板不影响任务；材料不被当作模板执行，替换值不递归执行；resume 沿用原渲染结果 |
| A48 | 原始结果与字段提取 | raw 保留最终响应原文；result.report 可提取 JSON 字段或 XML 元素，缺失/歧义/解析错误非零退出；--json 校验提取结果，--format json 再包装；重定向结果一致，result 可从保存原文重新提取且不调用模型 |
| A49 | 日志与单次请求超时 | 四档 run_logs 只改变 stderr 详略，result 档不污染结果且保留必要失败诊断；llm_timeout 超时与命令总 timeout 耗尽原因明确区分，前者在总时长未耗尽时可恢复，后者为达到限制的终态 |

首版成功的标志是：程序员愿意把 xllm 频繁放进各种小脚本，输入和输出容易组合，执行边界及副作用可预期，失败能够及时被脚本识别，未完成的工作可按需恢复。使用过程中不要求理解内部请求哈希、快照文件、Rust 类型或底层调度器。

## 10. 与其它文档的关系

- 本文定义 xllm 的用户需求和首版产品范围。
- [现有 run_local_llm 目录与命令行协议](../../doc/agent_tool/local_llm_context_protocol.md) 用于理解已有能力及问题，不作为新产品的兼容约束。
- [现有 XML behavior 协议提示词](../../src/frame/llm_context/src/xml_behavior.rs) 和 [OpenDAN 模板环境](../../src/frame/opendan/src/prompt_env.rs) 是运行时协议装配的实现参考，不限定 xllm 通用 loop 的任务类型。当前协议文本通过模板变量引入，loop 本身不自动补入；新 SDK 须保证通用规则、实际协议说明与执行器成套装配。
- 后续 SDK/CLI 技术设计需要落实目录默认任务与显式选组、Provider 对象与身份来源、主模型/文件模型分工、两种 loop 与 tools2actions、MCP/内置工具解析、组内工具覆盖、外部组文件展开、环境模板及 section 行号拼接、result_format 提取与 CLI 包装、run_logs 与两类超时、配置来源和凭据引用、管道执行时序、错误分类与退出码、Runs 格式和恢复保证；遇到产品行为变化时，应同步修订本 PRD。
