# new opendan agent runtime

> 重构目标：把 opendan 从「自己写 Agent Loop / Behavior 解析 / step 记录」改造成
> 「**只负责构造正确的 LLMContextRequest + 正确的 LLMContextDeps，调度 LLMContext.run() / resume()，并消化 Outcome**」。
>
> 真正的 LLM 推理循环、tool dispatch、step 记录、错误自动反馈、快照/resume，
> 全部下沉到 `llm_context` crate（slim-waist 已经实现）。
> opendan 是这个 waist 之上的 L3/L4 调度器 + 持久化层。

## 核心架构原则（必须遵守）

1. **外部边界 = buckyos-api**：从系统获取 Message / Event 一律走 `buckyos-api` 类型
   （`MsgCenterClient` / `KEventClient`），不绕开 SDK，不自己造跨进程协议。
2. **内部 dispatch = tokio 队列基础设施**：进程内的输入传递、worker 唤醒、shutdown 信号一律
   用 tokio 原语（`mpsc` / `Notify` / `select!`），不自己造调度器。
3. **AgentSession 是状态管理的核心**：任何"已经从系统取走、但还没被 LLM 真正消费"的 msg / event
   必须落到 `AgentSession.meta.pending_inputs` 持久化字段里；session 自己在 worker loop 的合适
   状态下从 pending queue 取出消费。
4. **ack 上游 ↔ session 持久化对齐**：
   - 落盘前进程崩 → msg-center 仍是 `Reading` → 下次 boot 重新拉同一条
   - 落盘成功 → 立刻 `update_record_state(Readed)` ack 给 msg-center
   - 这条 invariant 决定了 pump / dispatcher / session 三方的分工（见 §4 伪代码）。

## 运行时部署模型（前置假设）

整套设计建立在下面这条"数据 / 运行时"分离上：

| 维度 | AgentRootFS（数据） | AgentRuntime（执行） |
|------|--------------------|--------------------|
| 形态 | 宿主机普通文件系统目录 | Linux Docker 容器 |
| 位置 | `/opt/buckyos/data/home/$userid/.local/share/$agentid/`（Instance Volume，宿主机视角） | 容器 mount Instance Volume + 临时卷 |
| OS 跨度 | Windows / macOS / Linux 宿主机均可承载，目录结构平台无关 | **始终 Linux**（bash + tmux + POSIX symlink 是刚性依赖） |
| 生命周期 | 长期持久化，跨容器重建保留 | 可随时销毁重建，无状态 |

**推论**（后面章节默认成立，不再重复）：

- AgentRootFS 是 Agent 的核心状态——目录结构、配置、session 数据、工具声明全部以平台无关
  的方式存在宿主机文件系统里，可在宿主机之间原样拷贝迁移。
- AgentRuntime 跑在 Linux 容器内，所以容器内用 POSIX symlink、`/opt/buckyos/...` 绝对路径、
  bash wrapper script 不需要做跨平台兼容，**只要保证执行视图在容器内可重建即可**。
- 跨平台兼容性的要求只落在 **AgentRootFS 的数据**上（容器内生成的派生物可以是纯 Linux 形态）。
- "session 不带 `./bin/` symlink"的根因不是"宿主机可能没有 symlink"——而是"绑死 Linux 镜像
  内部路径会让数据脱离镜像后失去意义"。即使宿主是 Linux，symlink 指向容器内路径也是反模式。

## 0. 与新 llm_context 的接口契约

opendan 通过两个对象与 waist 交互：

- **`LLMContextRequest`**（不可变输入，[request.rs](src/frame/llm_context/src/request.rs)）
  - `owner: ContextOwnerRef::Agent { session_id }`
  - `input: Vec<AiMessage>` — 已渲染的完整对话（system+user+history），不再有"模板"概念到达 waist
  - `model_policy / tool_policy / output / budget / human_policy / error_policy`
  - `tool_policy.mode` + `whitelist` 决定该步允许的工具集（用于 behavior 切换时收窄/放开权限）

- **`LLMContextDeps`**（运行依赖，[deps.rs](src/frame/llm_context/src/deps.rs:195)）
  - `llm: LlmClient` — 由 `aicc_client` 适配
  - `tools: ToolManager` — 由 opendan 的 `AgentToolManager` 适配（4 层 bin 合成在这里完成）
  - `policy: PolicyEngine` — `AgentPolicy`：基于 behavior_cfg 做 gate
  - `worklog: WorklogSink` — opendan 把 `WorkEvent` 翻译成 `WorklogService` 写盘 + 更新 session "一句话状态"
  - `tokenizer: Tokenizer` — 选 ByteHeuristic 起步
  - `turn_hook: TurnHook` — **每次推理前**回调，opendan 在这里把 `LLMContextSnapshot` 落盘到 `session/.meta/$state`
  - 三个可选项决定是「Agent Loop」还是「Behavior Loop」：
    - `result_parser: LLMResultParser` — opendan 的 `XmlBehaviorParser`（待实现，对应空的 `xml_behavior.rs`）
    - `step_renderer: StepRenderer` — 把 `StepRecord` 还原成 `(assistant, user)` 对喂回下一轮推理
    - `history_compressor: HistoryCompressor` — 可选，长 step 历史压缩

**结论：opendan 不再实现 LLM 循环本身**，只实现这 7 个 trait 的"opendan 风味"版本，以及围绕 session 的调度。

---

## 1. 状态分层

### AgentRuntime（进程级，单例）

waist 的 deps 公共依赖 + 边界客户端：
- `aicc_client: Arc<AiccClient>` — 适配为 `LlmClient`
- `worklog: Arc<WorklogService>` — 全局 SQLite 句柄
- `msg_center: Option<Arc<MsgCenterClient>>` — 边界：msg-center get_next / update_record_state
- `kevent_client: Option<Arc<KEventClient>>` — 边界：订阅 `/msg_center/{owner}/box/**` 等模式
- `contact_mgr`（TODO）— 给 forward_msg / forward 类工具用
- `task_mgr`（TODO）— 异步工具结果回填、跨 session 任务通知

`msg_center` / `kevent_client` 为 `Option` 是为了让 CLI / 单测可以在不连 zone 服务的情况下
跑 AIAgent — 这时 inbox 退化成「只接受 `submit_text` 注入」模式（见 §9.6 进度）。

### Agent（AgentRootFS，对齐 paios 容器需求 §9）

AgentRootFS 是**宿主机普通目录**（见"运行时部署模型"），位置按 paios 契约：
`/opt/buckyos/data/home/$userid/.local/share/$agentid/`（宿主机视角的 Instance Volume 源路径，
会被 mount 进 Linux 容器供 AgentRuntime 读写）。目录内布局（Agent Bin 层落在这里）：

```
/role.md + /self.md                      # 自我介绍，进 system prompt
/users/$user_id.md | group_$gid.md       # 针对调用者的系统提示词片段
/memory/                                 # AgentMemory 模块初始化
/notepads/$notepadname/                  # 多本 notepad，AgentMemory 初始化
/skills/$category/$skill_dir/            # Agent 加载的真实 skills（可 self-improve）
/tools/                                  # Agent 自写脚本工具（§2 4 层 bin 中的 Agent Bin 层）
/behaviors/$name.toml                    # Behavior 模板（系统提示词 + 允许工具 + parser/renderer 配置）
/archive/skills                          # 导入原始 skills，Agent 不直接看
/archive/sessions/$session_id            # 已归档 session
/archive/workspace/$workspace_id         # 已归档 workspace
/archive/worklog.db                      # SQLite 归档
/workspace/$workspace_id/                # 工作区目录
/workspace_list.md                       # 最近活跃 workspace 列表，有大小上限
/sessions/$session_id/                   # session 目录（含 .meta/session.json、.meta/state.snap）
```

**Session Bin 层不在 Agent Root 内、也不是 session 目录里的可持久化产物**——按 paios 契约
在容器内落在 `/opt/buckyos/tools/$agentid/$sessionid/`（rwx 卷），由启动器在 **session 启动时**
按 manifest 动态渲染：可以用 Linux symlink 或 wrapper script 指向 paios 镜像内置工具、
授权的 ExtTool Volume 工具、或 session 自带的临时工具。它是**可删除、可重建的派生产物**，
容器删除后即失效。System / Runtime Bin 层同样在 `/opt/buckyos/tools/` 下（`store/` + `bin/`），
见 §2 与 §9.2 残项。

**核心原则**：session 目录只保存"工具声明 + 素材"（平台无关，归 AgentRootFS / 宿主机），
paios Linux 容器在启动时生成"临时执行视图"（PATH 里那个 bin，归 AgentRuntime）。
这是相对旧版 opendan 把 session-bin 放在 `session/<sid>/.tool` 且含指向镜像内部路径 symlink
的破坏性变化——后者会污染 session 数据、绑死镜像版本、阻碍宿主机间迁移。
（数据 / 运行时分离的前提见"运行时部署模型"。）

### Workspace（local）

代表 Agent 的私有工作区。**workspace 优先拥有 task**（task 跟着 workspace 走，session 只是其执行载体）。

```
./.workspace.json     # 结构化状态，含与 session 的绑定关系
./readme.md           # 目录结构说明，会作为环境上下文片段进入提示词
```

参考现有 `LocalWorkspaceManager` ([local_workspace.rs](src/frame/opendan/src/local_workspace.rs))；
新 runtime 保留其数据模型（`WorkshopWorkspaceRecord` / `SessionWorkspaceBinding`），
但 session 绑定改由 `AgentSession` 自己持有引用，不再走全局 mgr 的 in-memory 快照。

### AgentSession

```
./.meta/session.json   # session 元信息：id / agent_did / owner / current_behavior /
                       #   status / one_line_status / pending_inputs[]
./.meta/state.snap     # 最新 LLMContextSnapshot（由 turn_hook 写入）
./.meta/state.$N.snap  # 历史快照，按 behavior 切换时归档
./readme.md            # session 目录说明，进环境上下文
./tools/               # session 级别工具的【声明 + 素材】，两种形态自由混用：
                       #   - 扁平：query_weather.ts / parse.sh / dedup.py 直接丢进来
                       #   - 结构化：summarize_pdf/{tool.toml, summarize.sh, prompts/...}
                       #   只放文本（脚本 / TOML / Schema / Prompt），禁止二进制；
                       #   平台无关、跨 OS 可迁移，不含可执行 symlink、不指向镜像路径
                       #   —— 详细声明规则见 §1.5
./report.md            # worksession 完成后的工作报告
./archive/             # 完整 history（包括 worklog 子集），可翻看
```

> **不要在 session 目录里放 `./bin/`**：`./bin/` 形态的 Linux 可执行目录 + 指向 paios 镜像
> 内部路径的 symlink 是反模式——它会让 session 数据绑死特定镜像版本、阻碍跨平台迁移、
> 让 Agent 有机会 link 到镜像或宿主机路径绕过授权。**所有"进 PATH 的 bin"都在容器临时目录里
> 由启动器渲染**（见 §1 Agent Root 段 / §2 Session Bin 层物理路径契约）。

`pending_inputs` 是「核心原则 #3」落在持久化层的字段，存的是 `enum PendingInput { Msg, Event }`
（见 [agent_session.rs](src/frame/opendan/src/agent_session.rs) `PendingInput`）。
写入路径走 `AgentSession::enqueue_pending(input)`：append → `flush_meta()`（tmp + rename
的 crash-consistent 写法）→ 唤醒 worker。worker 在 turn 成功后才从 pending_inputs 里删除已消费项
并 flush_meta，失败则保留以便重启 / 下次唤醒重放（at-least-once）。

**Session 类型**：
- **UI Session**：永远活跃，每个 UI tunnel 对应一个；天然带 `try_create_worksession` / `forward_msg` 等工具
- **Work Session**：状态机，非 END 状态下都算活跃；由 UI session 用 `try_create_worksession` 派生

---

## 1.5 Session Tool 渲染契约

本节定义"session 工具声明 → 容器内 PATH 上可执行 bin"的完整转换。术语收敛：

- **render plan**：把 session 所有授权与声明合并展开后得到的工具列表，是渲染唯一输入。
- **执行视图**：`/opt/buckyos/tools/$agentid/$sid/` 下的物理文件（symlink + wrapper script）。
- **逻辑视图**：`AgentToolManager::list_tool_specs()` 喂给 LLM 的工具描述列表。
- 两个视图**同源于 render plan**，由启动器先算 plan、AgentToolManager 反向读 plan 生成 spec。

### 1.5.1 数据来源（source of truth）

render plan 由两类输入合成：

1. **`session.json.tool_grants[]`** — session 启动时由策略层（用户授权 + Agent 默认）写入，
   LLM 不可改。每条形如：
   ```json
   {
     "id": "g_ffmpeg_v1",
     "target_layer": "runtime",    // system | runtime | exttool
     "target_name": "ffmpeg",
     "exec_alias": "ffmpeg",       // 可选；执行视图下用的命令名（默认 = target_name）
     "granted_by": "user|agent_default",
     "granted_at": "2026-05-14T..."
   }
   ```
   - `target_layer = "system"` 的 grant 可省略，启动器默认把 System Bin 全量授予所有 session。
     需要收窄时才显式写。
   - Agent Bin 同理默认全量授予该 Agent 名下所有 session（同 Agent 内可信）。
2. **session 自带工具声明**，支持两种形态（启动器都识别）：

   **(a) 扁平脚本形态** — 直接把脚本文件丢进 `./tools/` 即可。这是日常 prototype / 单文件脚本
   的默认入口，LLM 写一个 `query_weather.ts` 进去就算交付了。
   ```
   ./tools/query_weather.ts
   ./tools/parse_invoice.sh
   ./tools/dedup_csv.py
   ```
   元数据自动推断：
   - `name` = 文件名去扩展名（`query_weather.ts` → `query_weather`，禁止重名）
   - `interpreter` = 按扩展名映射（`.sh|.bash` → bash，`.py` → python3，`.ts` → tsx/bun，
     `.js|.mjs` → node，`.rb` → ruby）；扩展名无映射时按 shebang；都没有则报 warn 跳过
   - `description` / `input_schema` = 从文件头 docblock 提取（约定 `# @description: ...` /
     `# @input_schema: { ... }` 或顶部 `/** ... */` 块）；缺省时给 LLM 一句兜底
     `"user-defined script: <name>"`，schema 留空（LLM 自行决定 argv / stdin JSON）
   - `version` = 文件 mtime 哈希（仅用于 plan 缓存判定，不喂给 LLM）

   **(b) 结构化形态** — 需要显式 schema / 多文件 / `ref` 类引用上层工具时用：
   ```
   ./tools/summarize_pdf/tool.toml
   ./tools/summarize_pdf/summarize.sh
   ./tools/summarize_pdf/prompts/sys.md
   ```
   `tool.toml` schema：
   ```toml
   name = "summarize_pdf"        # 必须与目录名一致
   version = "0.1.0"
   description = "..."            # 给 LLM 看的说明
   input_schema = '''{...}'''     # JSON Schema 字符串，喂给 LLM

   [source]
   kind = "script"                # script | ref
   # kind = "script" 时：
   entry = "summarize.sh"         # 相对当前 tool 目录
   interpreter = "bash"           # 可选；默认按 shebang
   # kind = "ref" 时：
   # target_layer = "agent"       # system | runtime | agent | exttool
   # target_name  = "actual_name"
   # grant_ref    = "g_xxx"       # target_layer ∈ {runtime, exttool} 必填，指回 tool_grants
   ```

   两种形态的共同约束：
   - 都没有 grant 概念——脚本就在 session 数据里，能写它的人已经能 `exec_bash`。
   - 扁平形态发现规则：扫描 `./tools/` 顶层"文件"；扫描 `./tools/` 顶层"目录"则按结构化形态
     处理（识别 `tool.toml`）。扁平脚本和同名子目录冲突时拒绝 render 并要求 LLM 改名。
   - LLM 可以从扁平形态"升级"到结构化形态：把 `query_weather.ts` 移到 `query_weather/` 目录
     并写一份 `tool.toml`——启动器把这视为同一个工具的演化（name 一致即可）。
   - **`./tools/` 只放文本**：解释执行脚本（.sh/.py/.ts/.js/.rb/...）、tool.toml、prompts、
     JSON Schema、README 等。**禁止任何二进制**（ELF / Mach-O / PE / .so / .dylib / .dll /
     编译产物 / 打包的二进制 wheel-with-native 等）。理由：
     - 二进制天然平台绑定，破坏 AgentRootFS 的跨宿主机可迁移性
     - 二进制无法被 LLM / 审计者直接阅读，等于在 session 数据里夹带不透明产物
     - 真正需要的本地工具走 `tool_grants` 引用 Runtime Bin / ExtTool Volume，那里有版本与
       授权治理；session 不是分发二进制的渠道
     启动器在 render 时按 magic bytes 探测：发现二进制文件 → 跳过该工具并 log warn，不阻断
     其他工具渲染。
   - **`./tools/` 必须保持小且文件数有限**：这是 hot path。`exec_bash`（以及任何能写文件的
     session 工具）执行后，启动器都可能对 `./tools/` 做一次 sync 检查（默认实现是 mtime
     walk，必要时升级到内容 hash 触发 re-render，见 §1.5.6）。所以：
     - 单文件建议 ≤ 64 KB（脚本本身轻小，prompt/schema 长就拆 include）
     - 整个 `./tools/` 文件数建议 ≤ 几百，**不要把数据集、日志、抓取结果、模型权重、缓存等
       塞进来**——这些属于 session 根、workspace 或 `./archive/`
     - 一次 walk 慢于 50ms 时启动器会 log warn，超过阈值时 sync 退化为"turn 边界一次"而不是
       "每条 bash 后一次"，session 工具的热更新感会变差
     LLM 在写新工具时如果发现 entry 脚本超过 64 KB，应该拆素材到 prompts/ 子目录走结构化形态
     而不是塞进扁平脚本。

### 1.5.2 render plan 合成

启动器执行如下顺序，先到先得（同名后者忽略，并记 warn）：

1. session 自带工具（同时扫描两种形态，最高优先级）：
   - `./tools/<dir>/tool.toml` 结构化形态
   - `./tools/<file>.{sh,bash,py,ts,js,mjs,rb,...}` 扁平形态
   - 同 session 内 name 冲突 → 拒绝 render
2. tool_grants 中 `target_layer = "exttool"`、`"runtime"` 的条目
3. Agent Bin（遍历 Agent Root `/tools/`，同样支持扁平与结构化两种形态）
4. tool_grants 中 `target_layer = "system"` 显式条目；若无，则枚举 System Bin 全量

产出（不落 session，写到容器内临时区 `/opt/buckyos/tools/$aid/$sid/.render_plan.json`）：

```json
{
  "session_id": "sid_xxx",
  "rendered_at": "2026-05-14T...",
  "exec_dir": "/opt/buckyos/tools/$aid/$sid",
  "entries": [
    {
      "name": "summarize_pdf",
      "exec_path": "/opt/buckyos/tools/$aid/$sid/summarize_pdf",
      "kind": "wrapper",
      "wrapper_body": "#!/bin/bash\nexec /opt/buckyos/data/.../sessions/$sid/tools/summarize_pdf/summarize.sh \"$@\"\n",
      "spec_source": "session_script",
      "description": "...",
      "input_schema": "..."
    },
    {
      "name": "ffmpeg",
      "exec_path": "/opt/buckyos/tools/$aid/$sid/ffmpeg",
      "kind": "symlink",
      "target": "/opt/buckyos/tools/bin/ffmpeg",
      "spec_source": "runtime_bin",
      "grant_ref": "g_ffmpeg_v1"
    }
  ]
}
```

### 1.5.3 物理渲染

启动器拿 plan 在 `exec_dir` 下做这些操作（顺序敏感）：

1. `rm -rf <exec_dir>/*`（执行视图无状态，整目录清掉重建）。
2. 对每个 `entries[i]`：
   - `kind = "symlink"`：`ln -s <target> <exec_path>`
   - `kind = "wrapper"`：写出 `<exec_path>` 内容 = `wrapper_body`，`chmod +x`
3. `.render_plan.json` 最后写——AgentToolManager 看到这个文件才认为视图就绪。

### 1.5.4 容器挂载与路径

| 卷 | 宿主机源 | 容器内路径 | 权限 |
|----|---------|----------|------|
| Instance Volume | `/opt/buckyos/data/home/$uid/.local/share/$aid/` | 同源路径（透传） | rwx（容器用户） |
| Worker Image Tools | 镜像内置 | `/opt/buckyos/tools/store/` | rx |
| Runtime Bin | 容器临时卷 | `/opt/buckyos/tools/bin/` | rx |
| Session Exec View | 容器临时卷 | `/opt/buckyos/tools/$aid/$sid/` | rwx |
| ExtTool Volumes | 用户挂载 | 由 grant 指定 | rx |

约束：因为宿主源和容器路径同源，wrapper script 里 `exec` 后面可以直接写
`/opt/buckyos/data/.../sessions/$sid/tools/<name>/<entry>`，启动器和容器内进程看到的是同一条
绝对路径。

### 1.5.5 AgentToolManager 与启动器的分工

| 关注点 | 启动器（容器编排层） | AgentToolManager（waist 实现，容器内） |
|--------|------------------|------------------------------------|
| 输入 | `session.json.tool_grants` + `./tools/*/tool.toml` + Agent Root + 镜像 | `<exec_dir>/.render_plan.json` |
| 产出 | render plan + 物理执行视图 | `Vec<ToolSpec>` + dispatch 闭包 |
| 写入文件 | `exec_dir` 下所有文件 | 无（只读 plan） |
| 调用工具 | 不调用 | `Command::new("<exec_path>")` 或直接 path 入 `$PATH` |

硬约束：**AgentToolManager 不准枚举 `Agent Root /tools/`、`/opt/buckyos/tools/store/` 等真实
来源**——唯一允许读的就是 `.render_plan.json`。这条约束保证 LLM 视角和 PATH 视角不可能 drift。

### 1.5.6 渲染时机

| 触发点 | 动作 | 是否打断推理 |
|--------|------|----------|
| session worker cold start / resume | 同步 render，未完成不进 run loop | 启动期，无推理可打断 |
| `exec_bash` 返回后 | 对 `./tools/` 做 sync 检查（mtime walk）；有改动才标记 dirty | 否，是 bash 工具调用尾部的一步 |
| 显式写工具的 session 工具（`write_file` / `edit_file` 等命中 `./tools/`）| 直接标记 dirty | 否 |
| `tool_grants` 变化（用户授权 / 撤权）| 标记 dirty | 否 |
| 任一 dirty 触发后，**下一个 turn 边界** | re-render | 否，turn 间空档 |
| behavior 切换 | **不 re-render**，只调 `tool_policy.whitelist` gate | — |

实现要点：

- `AgentSession` 持有 `tools_dirty: AtomicBool` + `tools_mtime_snapshot: HashMap<PathBuf, SystemTime>`。
- `exec_bash` 工具实现在返回前 walk `./tools/` 顶层 + 一层子目录的 mtime，diff 上一份快照；
  有差异才 `tools_dirty.store(true)`，并刷新快照。
- worker 在每个 turn 入口检查 dirty，若为 true 则调启动器暴露的
  `re_render_session_tools(sid)` RPC（启动器进程外，AgentRuntime 进程内 client），完成后清 flag。
- walk 慢于阈值（默认 50ms）→ log warn；session 工具数 / 总大小逼近上限（§1.5.1）时切换到
  "turn 边界统一一次 walk"模式，牺牲热更新换性能。

### 1.5.7 待决策（开发前确认）

1. **是否允许 session script 反向引用 Agent Bin** — 当前设计允许（`kind = ref` +
   `target_layer = agent`，无需 grant，因为同 Agent 内默认互信）。如果安全模型要求 worksession
   与 UI session 互相隔离，需要为 Agent Bin 也加 grant。
2. **render plan 是否需要签名 / hash** — 当前不签。如果担心容器内进程篡改 plan 后越权调用，
   可让启动器在 plan 里加 HMAC，AgentToolManager 启动时校验。
3. **session `./tools/` 命名冲突** — 当前 session 自带工具覆盖上层；可考虑反过来（拒绝
   覆盖 + 强制 LLM 改名），更保守。
4. **`tool_grants` 撤权时正在执行的工具调用** — 当前设计：撤权只影响下次 render，已经在执行
   的子进程不杀；要不要硬切？

---

## 2. AgentTool 4 层合成

`AgentToolManager::list_tool_specs()` 返回的工具集是 4 层合并的结果（同名后者覆盖前者）：

| 层 | 范围 | 来源 | 权限 |
|----|------|------|------|
| System Bin | 所有 Agent 可见 | BuckyOS 发行镜像 | 只读 |
| Runtime Bin | 特定 Agent 可见 | 用户安装到 Agent 工具卷的二进制 | 通常只读，按权限放开 |
| Agent Bin | 特定 Agent 可见 | Agent 自己写的脚本（在 `/tools/`） | Agent 可修改 |
| Session Bin | 特定 session 可见 | Session 启动时按权限创建（软链接 + 脚本） | Session 内可修改 |

合成发生在 **`AgentToolManager` 构造 / 每次 session 启动** 时，结果缓存在 manager 内部，
对 waist 暴露统一的 `ToolManager` 接口。`tool_policy.whitelist` 在 behavior 切换时控制可见子集（不重新合成，只 gate）。

**4 层 bin 的物理路径契约**（来自 [paios 容器需求.md §9](paios容器需求.md)）：

| 层 | 路径 | 权限 | 承载 | 持久化 |
|----|------|------|------|--------|
| System Bin | `/opt/buckyos/tools/store/` | rx，所有 App 共享 | Worker Image 预置 CLI（ffmpeg/pandoc/...） | 随镜像 |
| Runtime Bin | `/opt/buckyos/tools/bin/` | rx，App-scoped symlink view | 从 `store/` + ExtTool Volume 渲染（Crafter 镜像产出工具包接入处） | 容器临时，按 manifest 重建 |
| Agent Bin | Agent Root 下 `/tools/`（Instance Volume 内） | rwx 给 Agent | Agent 自演化脚本；升级走文件级合并（paios §7.4 R-15） | 持久化（Instance Volume） |
| Session Bin（声明） | session 目录 `./tools/` | rwx | tool.toml / 脚本源 / 工具包元数据 / 交付物（平台无关） | 持久化（跨 OS 可迁移） |
| Session Bin（执行视图） | `/opt/buckyos/tools/$agentid/$sessionid/` | rwx | 启动器按 manifest 渲染的 symlink / wrapper script，指向 System / Runtime Bin 工具、授权的 ExtTool Volume 工具、或 session `./tools/` 下的脚本 | 容器临时，session 启动时重建 |

**渲染规则**（2026-05-14 修订：改为 PATH 拼接 + Session 渲染的极简模型）：

- **System / Runtime / Agent 三层**：每层就是一个普通的 bin 目录，**不做任何渲染**，
  直接按顺序拼到 PATH 即可。系统工具（ffmpeg/pandoc/...）、卷工具（ExtTool Volume）、
  Agent 自写脚本，都是"现成就摆在 bin 里"的文件。
- **Session Exec Bin 是唯一需要"渲染"的层**，承担两件事：
  1. **Agent tools 落地**：session 启动时把 Agent Root FS 的 `/tools/` 内容自动拷过去（一份
     可写副本），session 内的修改不污染 Agent 持久层；session 结束时这份拷贝丢弃。
  2. **墓碑效应（tombstone）**：session 启动时读 plan 文件（白名单 / 黑名单），在 Session Exec Bin
     里放 stub wrapper 屏蔽下层的同名工具——例如要禁用 ffmpeg，就在 Session Exec Bin 里写一个
     `ffmpeg` 脚本 `exit 127`（或返回友好错误），PATH 优先级保证它先被找到。
- session 的 `./tools/` 只放工具的"声明 + 源文件 + 元数据"——属于 AgentRootFS（宿主机数据层），
  必须能在不同宿主机之间原样拷贝；执行视图由目标宿主机上启动的 Linux 容器在启动时重新渲染
  （执行视图本身始终是 Linux 形态，不需要跨 OS 兼容）。
- 容器删除/迁移时 Session Exec Bin 整个丢弃，下次启动重建。

PATH overlay 顺序：**Session > Agent > Runtime > System**（前者优先，同名覆盖；用 PATH 拼接而非
统一渲染实现）。

**UI Session 默认工具集**（写入 `behaviors/ui_default.toml` 的 whitelist）：
- `exec_bash` / `read_file` / `glob` / `grep` / `edit_file` / `write_file`
- `try_create_worksession { reason }` — fork 出 sub-LLMContext，基于近况和 worksession
  列表决定复用已有 / 新建。最终由 sub-context 调 `create_worksession` 落地，结果原样回传给
  UI session。详见 §8.1–§8.3。
- `forward_msg { target_worksession_id }` — 把"触发本轮推理的最近 user 消息"作为
  `PendingInput::Msg` 派发到指定 worksession（进程内路由，不走 msg-center）。详见 §8.4。
- `update_session_tags` — 主动触发一次 memory 召回

---

## 3. Behavior Config

每个 behavior 是一份配置（建议 TOML，落在 `/behaviors/$name.toml`）：

```toml
name = "ui_default"
system_prompt_template = "..."        # 引用 role.md / self.md / users/*.md 的渲染模板
tool_whitelist = ["exec_bash", "read_file", "try_create_worksession", "forward_msg"]

# Behavior 模式（决定 LLMContextDeps 是否装 parser/renderer）
mode = "behavior"                     # "agent" | "behavior"
                                      # agent: 走传统 Agent Loop（provider 原生 tool_calls）
                                      # behavior: 装上 parser+renderer，走 Behavior Loop

# Parser / Renderer 选择（mode = "behavior" 时生效）
parser = "xml"                        # 默认 "xml" → llm_context::XmlBehaviorParser
renderer = "xml"                      # 默认 "xml" → llm_context::XmlStepRenderer
parser_strict = false                 # XmlBehaviorParser.strict：true 时纯文本回复算错误

# Renderer 调参（XmlStepRenderer 的旋钮，全部可选）
renderer_recent_full_steps = 2        # 最近 N 步全量渲染；更老的步骤压缩
renderer_summary_chars = 280          # 压缩步骤的 assistant_text 字符上限
renderer_max_result_chars = 4096      # 单步 action_result 上限（0 = 不截断）

# 输出与预算
output = "text"                       # "text" | "json"（json 时需在 output_spec 里给 schema）
max_rounds = 16                       # ToolPolicy.max_rounds
max_consecutive_errors = 3            # ErrorPolicy.max_consecutive_errors

# Behavior 切换语义
switch_mode = "normal"                # "normal" | "fork" | "independent"
```

opendan 的 `agent_config` 负责把这份 TOML 翻译成 `LLMContextRequest` + `LLMContextDeps`
（往 `deps.result_parser` / `deps.step_renderer` 装入对应实现）。当 `mode = "behavior"` 时，
默认装 `Arc<XmlBehaviorParser>` 和 `Arc<XmlStepRenderer>`（已实现，见下）。

### 默认 XML Behavior 协议（已实现）

系统默认实现已落在 `llm_context` crate 里，opendan 直接 `use` 即可：

- [`llm_context::XmlBehaviorParser`](src/frame/llm_context/src/xml_behavior.rs) — `impl LLMResultParser`
- [`llm_context::XmlStepRenderer`](src/frame/llm_context/src/step_record.rs) — `impl StepRenderer`

**LLM 输出的 wire format**（每个 tag 都可选；外层 `<response>` 也可选，存在时收窄扫描范围）：

```xml
<response>
  <thinking>...自由形式推理...</thinking>
  <observation>...对上一步 action_result 的解读...</observation>
  <action tool="exec_bash" call_id="optional">
    {"command": "ls -la"}
  </action>
  <next_behavior>END</next_behavior>
</response>
```

`<action>` 体的解析规则：
- Body 解析为 JSON 对象 → 作为 `args`
- 否则 → `args["content"] = body`（保持原文）
- 非保留属性（`tool` / `name` / `call_id` 之外）作为字符串 args 注入
- Provider 原生 `tool_calls`（function-calling）优先于 `<action>` 扫描

**强容错**（无需额外旁路 LLM 修复就能 cover 大多数 case）：
- 剥离 ```` ```xml ```` / ```` ``` ```` markdown fence
- 缺失 close tag 时从开标签取到 EOF
- 属性值支持双引号/单引号/无引号；5 个 XML 实体（`&amp;` 等）会解码
- 整段 response 没有可识别结构时，`assistant_text` 原样保留，被当作"自然收敛终态步骤"
- 真正失败的（完全空 response，或 `parser_strict=true` 且既无 action 又无 next_behavior）才返回 `Err`，
  由 waist 自动合成一条 error step 喂回 LLM 自我纠正

**Renderer 行为**：
- `render(step) → (assistant, user)`：assistant = verbatim `assistant_text`；user = `<action_result tool="X" call_id="Y" status="ok|error|pending"[ truncated="true"]>BODY</action_result>`（或 `<step_ack/>` 当 step 无 action）
- `render_history(steps)`：两档压缩——最近 `recent_full_steps` 个步骤全量；更老的步骤用 `<thinking>summary</thinking>` 形式收敛 assistant_text，action_result body 截断
- 严格保持 `(assistant, user)` 交替，对所有 provider 都兼容

**自定义实现**：worksession 想用别的协议（JSON 行、ReAct markdown、自定义 DSL……）时，
实现 `LLMResultParser` + `StepRenderer` 两个 trait 装到 `deps` 即可，不需要改 waist。

### Behavior 切换的两种模式（switch_mode）

> 设计实施过程中（2026-05-14 第 3 轮，见 §9 当前进度）发现 fork 不适合做 `next_behavior` 触发的切换——fork 子 ctx 没法从父 ctx 的 `Done` 状态干净接管。最终落地：
> - **`switch_mode` 只有 `normal` / `independent`** 两个值（cfg 里仍保留 `fork` 这个 enum 值但 fall-through 到 normal + warn，等迁移完毕清掉）
> - **fork 是 session 内部原语**，触发口在 session-aware 工具的 `call()` 实现（典型：`try_create_worksession`），LLM 视角下是普通 tool 调用
> 详见 [llm_context_helper.rs](src/frame/opendan/src/llm_context_helper.rs) 设计草案的"三种模式的核心语义"段。

- `normal`：单一全局 step record stream，多个 behavior 在图上跳转。同一逻辑 session 内 system prompt 被替换、`request.input` 前导 System 段同步更新，全部历史保留。跨 behavior 跳转默认继承所有 step records；切到 B 再切回 A 仍能看到完整历史
- `independent`：每个 behavior name = 一个独立"进程"，**自己持有 step record stream**（持久化到 `.meta/behavior_<name>.snap`）。父→子是栈式（`SessionMeta.process_stack` 入栈），子 `END` pop 栈、控制流回父；父看不到子内部，但子下次被同名再切回时能续上自己之前的 stream
- ~~`fork`~~（不通过 `next_behavior` 触发；见上方说明）：父 ctx 在 fork 点挂起→子从父快照继承+可改 env→子 `END`→控制流回父，父只拿到子的 `ContextOutput`（类似函数 return value），看不到子的 step records；可嵌套

---

## 4. 顶层伪代码

数据流（满足核心原则 #1~#4）：

```text
buckyos-api (msg-center / kevent)            ← 边界
        │
        │  pull_event / get_next / update_record_state
        ▼
msg_center_pump.rs   (fetcher — 只翻译，不 ack)
        │
        │  tokio mpsc<Inbound>                ← 内部 dispatch
        ▼
AIAgent::dispatch_inbound
        │
        │  session.enqueue_pending(PendingInput)
        │     ├─ meta.pending_inputs.push(...)
        │     └─ flush_meta()  (落盘成功才返回 Ok)
        │
        ▼
AgentSession  (持久化状态中心)
        │
        ├─ enqueue 返回 Ok → AIAgent.ack_msg_record(record_id)
        │                       ↑
        │                       └─ msg_center.update_record_state(Readed)
        │
        ▼
session worker loop
        │  Idle / WaitingInput 状态下从 meta.pending_inputs 取
        │  run_one_turn 成功 → discard_consumed(keys) + flush_meta
        │  run_one_turn 失败 → 保留在 pending_inputs，下次 Wakeup 重放
```

### 入口 + 分发（当前实现）

```rust
pub async fn AIAgent::run(self: Arc<Self>) -> Result<()> {
    self.restore_active_sessions().await;        // 重建非 Ended 的 session
                                                 // 每个 session worker 启动后会自动消费其 pending_inputs
    let pump = self.spawn_msg_center_pump();     // 只在 msg_center + kevent_client +
                                                 // parseable agent_did 都齐时才 spawn
    loop {
        tokio::select! {
            item = self.inbox_rx.recv() => match item {
                Some(it) => self.dispatch_inbound(it).await?,
                None     => break,
            },
            _ = self.shutdown_rx.recv() => break,
        }
    }
    self.pump_shutdown.notify_waiters();
    if let Some(h) = pump { let _ = h.await; }   // 等 pump 把 EventReader close 干净
    self.stop_all_sessions().await;
    Ok(())
}

async fn dispatch_inbound(&self, item: Inbound) -> Result<()> {
    match item {
        Inbound::Msg { record_id, from, session_id, text } => {
            let sid = session_id.unwrap_or_else(|| self.resolve_ui_session(&from));
            let session = self.get_or_create_session(sid, from.clone()).await?;
            session.enqueue_pending(PendingInput::Msg { record_id: record_id.clone(), from, text }).await?;
            self.ack_msg_record(record_id).await;   // 落盘后才 ack 给 msg-center
        }
        Inbound::Event { event_id, target_session_id, data } => {
            // MVP：只处理预路由的 event；session_sub_kevent 路由待补
            let Some(sid) = target_session_id else { warn!("event dropped"); return Ok(()); };
            if let Some(s) = self.session_by_id(&sid) {
                s.enqueue_pending(PendingInput::Event { event_id, data }).await?;
            }
        }
    }
    Ok(())
}
```

### msg-center pump（已实现，[msg_center_pump.rs](src/frame/opendan/src/msg_center_pump.rs)）

```rust
async fn run(cfg: PumpConfig) {
    let patterns = build_msg_center_event_patterns(&cfg.owner_did);
    let mut reader: Option<Arc<EventReader>> = None;
    loop {
        if reader.is_none() { reader = cfg.kevent_client.create_event_reader(patterns.clone()).await.ok().map(Arc::new); }
        let mut boxes = Vec::new();
        tokio::select! {
            _ = cfg.shutdown.notified() => { /* close reader; return */ }
            res = reader.as_ref().unwrap().pull_event(Some(1000)) => match res {
                Ok(Some(evt)) => collect_event_pull_targets(&evt, &mut boxes),  // 根据 eventid 选 BoxKind
                Ok(None)      => append_all_inbox_boxes(&mut boxes),            // 超时 → 全 inbox sweep
                Err(KEventError::ReaderClosed(_)) => { reader = None; append_all_inbox_boxes(&mut boxes); }
                Err(_)        => append_all_inbox_boxes(&mut boxes),
            }
        }
        for kind in boxes {
            // get_next(state=[Unread], lock_on_take=true, with_object=true)
            // 翻译成 Inbound::Msg 后扔进 cfg.inbox_tx —— 不在这里 mark Readed
            drain_box(&cfg, kind).await;
        }
    }
}
```

> **关于"每个活动 session 一个线程"**：保留——UI session 天然活跃；worksession 在非 END 状态时也活跃。
> 每个活动 session 一个 tokio task 跑 worker 循环，免去自写调度器，关闭/重启路径也简单
> （task abort + 从最新 snapshot resume + 重放 pending_inputs）。代价是空闲 session 也占一份
> task，但相比 LLM 调用成本可忽略。

### Session Worker（持久化队列消费模型）

```rust
// SessionInput 现在只是【唤醒信号】，载荷在 meta.pending_inputs 里
enum SessionInput { Wakeup, Cancel }

async fn AgentSession::run_worker(self: Arc<Self>, inbox_rx: &mut mpsc::Receiver<SessionInput>) {
    loop {
        // 1) 抢先消费 Cancel（不能被一个长 turn 卡住）
        while let Ok(Cancel) = inbox_rx.try_recv() { self.set_status(Idle).await; /* break if Work */ }

        // 2) 快照 pending —— 不在这里删，等 turn 成功再删
        let pending = self.meta.lock().await.pending_inputs.clone();
        if pending.is_empty() {
            match inbox_rx.recv().await {
                None | Some(Cancel) => return,
                Some(Wakeup)        => continue,
            }
        }

        // 3) 分流：Msg 喂 LLM；Event 在 MVP 阶段 warn 后丢
        let (texts, consumed_keys) = split_pending(&pending);
        if texts.is_empty() { self.discard_consumed(&consumed_keys).await; continue; }

        self.set_status(Running).await;
        match self.run_one_turn(texts).await {
            Ok(NextAction::Idle)        => { self.discard_consumed(&consumed_keys).await; self.set_status(Idle).await; }
            Ok(NextAction::WaitForMsg)  => { self.discard_consumed(&consumed_keys).await; self.set_status(WaitingInput).await; }
            Ok(NextAction::End)         => { self.discard_consumed(&consumed_keys).await; self.set_status(Ended).await; return; }
            Err(err) => {
                // 失败：保留 pending_inputs，等下次 Wakeup 重放 / 人工介入
                self.set_status(Error).await;
                // wait — 否则会 hot-loop 在同一 bad input
                let _ = inbox_rx.recv().await;
            }
        }
    }
}
```

```rust
// ===== 构造/恢复 LLMContext =====
async fn build_or_resume_context(&self, inputs: Vec<SessionInput>) -> LLMContext {
    let deps = self.make_deps();        // 见 §0；turn_hook = self.snapshot_writer

    // A) 有快照 → 优先 resume
    if let Some((snap, fill)) = self.try_make_resume_fill(&inputs).await {
        return LLMContext::resume(snap, fill, deps).expect("snapshot integrity");
    }

    // B) 新 session 或 behavior 切换后的全新 context
    let behavior = self.current_behavior_cfg();
    let mut messages = self.render_system_messages(&behavior).await;   // role.md / self.md / users/* / workspace/readme.md / session/readme.md
    messages.push(self.compose_environment_message(&inputs).await);    // "环境感知 message"：自动召回 memory + workspace/session 当前状态 + 新事件/新消息
    messages.extend(self.replay_visible_history().await);              // 历史片段（受限于压缩策略）

    let request = LLMContextRequest {
        owner: ContextOwnerRef::Agent { session_id: self.id.clone() },
        trace: Some(format!("{}::{}", self.id, self.next_trace_id())),
        objective: behavior.objective.clone(),
        input: messages,
        model_policy: behavior.model_policy.clone(),
        tool_policy: ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: behavior.tool_whitelist.clone(),
            max_rounds: behavior.max_rounds,
            ..Default::default()
        },
        output: behavior.output_spec(),
        budget: behavior.budget.clone(),
        human_policy: behavior.human_policy.clone(),
        error_policy: ErrorPolicy {
            mode: ErrorMode::FeedAsObservation,
            max_consecutive_errors: behavior.max_consecutive_errors,
        },
    };
    LLMContext::new(request, deps)
}
```

```rust
// ===== Resume 选型 =====
async fn try_make_resume_fill(&self, inputs: &[SessionInput])
    -> Option<(LLMContextSnapshot, ResumeFill)>
{
    let snap = self.load_latest_snapshot().await?;
    let fill = match (&snap.state.pending_tool_calls.is_empty(), inputs) {
        // 之前 yield 在 WaitInput → 把新到的 user/tunnel 消息打成 HumanInput
        (true, inputs) if inputs.has_human_msg() =>
            ResumeFill::HumanInput { message: inputs.compose_human_message() },
        // 之前 yield 在 PendingTool → 等到了 tool 结果
        (false, inputs) if inputs.has_tool_results() =>
            ResumeFill::ToolResults { results: inputs.take_tool_results() },
        // 崩溃恢复 / 启动后第一次唤起，没有 pending → ResumeFromMidRun
        (true, _) => ResumeFill::ResumeFromMidRun,
        // pending 不空但没收齐 → 不能 resume，继续等
        _ => return None,
    };
    Some((snap, fill))
}
```

```rust
// ===== Outcome 消化 =====
async fn handle_outcome(&self, outcome: LLMContextOutcome) -> NextStep {
    match outcome {
        LLMContextOutcome::Done { output, behavior_result, response, trace, .. } => {
            // UI session：转 MessageObject 发回 tunnel
            // Work session：写 report.md / append step history（其实 step 已经在快照里）
            self.commit_done(output, behavior_result, response, trace).await;

            // behavior_result.next_behavior 是 worksession 的状态机信号
            if let Some(next) = behavior_result.and_then(|r| r.next_behavior) {
                return NextStep::SwitchBehavior(next);
            }
            // 自然收敛
            if self.is_ui_session() { NextStep::WaitForMsg }
            else { self.classify_work_session_done() }   // END / WAIT_FOR_TASK / WAIT_FOR_MSG
        }

        LLMContextOutcome::WaitInput { snapshot, prompt_to_human, deadline_ms, .. } => {
            self.persist_snapshot(snapshot).await;          // turn_hook 已写过，这里只是覆盖确认
            self.show_prompt_to_human(prompt_to_human).await;
            self.set_deadline(deadline_ms);
            NextStep::WaitForMsg
        }

        LLMContextOutcome::PendingTool { pending, snapshot, deadline_ms } => {
            self.persist_snapshot(snapshot).await;
            self.task_mgr.dispatch_async_tools(pending);    // 等回填
            self.set_deadline(deadline_ms);
            NextStep::WaitForTask
        }

        LLMContextOutcome::ContextLimitReached { snapshot, accumulated, .. } => {
            // 这里走 opendan 自己的压缩器（不同于 behavior_loop 内部的 HistoryCompressor）：
            // 把 accumulated 重写后用 ResumeFill::RewrittenHistory 续跑
            let rewritten = self.compress_messages(accumulated).await;
            self.queue_rewritten_history(snapshot, rewritten).await;
            NextStep::Continue
        }

        LLMContextOutcome::BudgetExhausted { which, partial, .. } => {
            self.mark_one_line_status(format!("budget exhausted: {:?}", which));
            // 不写快照（这次推理算"失败"），等自动/手动重试
            NextStep::WaitForMsg
        }

        LLMContextOutcome::Error { error, .. } => {
            // §6 错误处理：waist 已经处理过 Recoverable（FeedAsObservation），
            // 走到这里就是真正不可恢复的异常
            self.mark_one_line_status(format!("error: {error}"));
            self.discard_pending_snapshot().await;
            NextStep::WaitForMsg
        }
    }
}
```

---

## 5. 运行跟踪 / 快照 / 一句话状态

新 runtime 把"运行跟踪"全部对齐到 waist 的 hook：

- **塞入新消息时**：opendan 在 `compose_environment_message` 阶段附加一条"环境感知 message"，
  包含自动召回的 memory、workspace 状态、上次到现在的事件/消息 diff。这条消息**在 waist 之外**构造，不属于 step。
- **压缩**：分两层
  - waist 内的 `HistoryCompressor`（behavior 模式下，step 维度，可选）
  - opendan 自己的消息压缩（响应 `ContextLimitReached`，message 维度，必须）
- **worklog hook**：`WorklogSink::emit(WorkEvent::...)` 中——
  - 每次 `LLMStarted` / `LLMFinished` / `ToolCallPlanned` / `ToolCallFinished` 都更新 session 的"一句话当前状态"（给 UI 看的）
  - 同时落到 `WorklogService` 的 SQLite
- **每次推理返回时**：
  - `TurnHook::before_inference` 已经在**下一次**推理前把当前快照写盘了（"no double-bill on crash"）
  - opendan 额外在 `Outcome` 落地时做一次终态快照（Done 终止；WaitInput / PendingTool / ContextLimitReached 的 snapshot 直接持久化）
- **取消**：
  - 标准取消 = session 进入 idle，下一轮 worker 检查到 cancel flag 后不再启动新 LLMContext
  - 强制取消 = abort tokio task + 用最新快照标 `aborted`，下次进 worker 时按用户意图决定是否 resume

---

## 6. 错误处理

waist 已经分了 `ErrorClass::Fatal` 和 `ErrorClass::Recoverable`，opendan 只关心两件事：

1. **解析错误**（XmlBehaviorParser 失败）走 waist 内部的"合成错误 step → 下一轮自我纠正"路径；
   opendan **只在** parser 内部做强容错（机械修复 → 旁路 LLM 修复 → 抛错让 waist 合成错误 step）。

2. **真正的异常**（`Outcome::Error`）：
   - aicc 链路不可用、tool dispatch 内部 panic、snapshot 损坏
   - opendan：不写快照（这次推理失败）、更新 session 一句话状态为"异常失败"、等自动重试或手动重试

**AgentTool 内部的所有异常都必须正常返回**（`Observation::Error { message }`），让 waist 走
FeedAsObservation——这是为了利用 LLM 的自我修复能力。

---

## 7. UI Session 结果回送

`Outcome::Done` 时，opendan 把 `ContextOutput::Text` 转成 `MessageObject` 发回原 tunnel。
WorkSession 的 `Done` 不需要特别处理——下游通过 worksession 的 `report.md` + worklog 拿结果。

---

## 8. WorkSession 工具：try_create_worksession / create_worksession / forward_msg

UI session 不直接构造 worksession——它经过一个 **fork 出来的 sub-LLMContext** 来决定
"复用已有 worksession 还是新建一个"，由 sub-context 调一个全参数的 `create_worksession`
落地。这套设计把"探索 / 选择"和"实际落地"拆成两个工具，加上 §8.4 的 `forward_msg`
组成 UI session 操纵 worksession 的全部入口。

### 8.1 `create_worksession`（全参数版，默认立即生效）

> 不暴露给 UI session 顶层；只出现在 `try_create_worksession` fork 出的 sub-context 白名单里。

```rust
create_worksession {
    title: String                        // worksession的标题，在某些场合会出现在worksesison list中
    objective: String,                   // 新 worksession 的目标
    workspace_id: Option<String>,        // worksession的bind的workspace None ⇒ 新建 workspace；Some ⇒ 复用已有
    behavior: Option<String>,            // 默认 = AgentConfig.default_work_behavior
    reason_message: Vec<String>,         // 描述意图的原始 message  
    auto_start: bool,                    // 默认 true；false ⇒ 只创建，不唤醒首轮执行
}
```

执行步骤：
1. workspace 解析 / 创建：`workspace_id = Some(id)` → `LocalWorkspaceManager::load_record(id)`；
   `None` → 生成新 workspace_id（建议 `ws-<ulid>`），`create_or_open(new_id, objective, ...)`
2. 创建 worksession 目录 + `.meta/session.json`：
   - 写入 `title` / `objective` / `behavior` / `workspace_id`（`SessionMeta` 需新增
     `title` / `objective` 字段；旧 JSON 用 `#[serde(default)]` 兼容）
   - 渲染 `readme.md`：包含 `title` / `objective` / 一段"起源消息（reason_message）"——
     按时序把 `Vec<String>` 拼成块状文本，worksession 推理时作为环境上下文片段进入
     system prompt（参考 §5 的"环境感知 message"）
3. `workspace.set_current_session(workspace_id, Some(new_session_id))` 绑定
4. `auto_start=true` 时**立即唤醒新 worksession**：`status = Idle` → 启动 worker → 因为 session.json 已有
   `objective`，worker 在 build_or_resume 阶段构造首轮 `LLMContextRequest` 时把
   objective 渲染进 system prompt 即可开始推理，**不需要外部消息触发**。这是 worksession
   与 UI session 的本质区别——它是"任务驱动"而非"对话驱动"的。
   - `auto_start=false` 时只创建并绑定 session/workspace，保持 Idle，不触发 bootstrap turn
   - 因此 `reason_message` 不进 `pending_inputs`：它只是 readme 里的起源凭据，objective
     才是工作驱动力
   - 这要求 worker loop 的"空 pending 等待"分支增加一种情形：work session + 有 objective
     且尚未跑过任何一轮 → 直接进 turn，而不是 block 在 `inbox_rx.recv()` 上（§4 伪代码
     需要相应小改）
5. 返回 JSON：
   ```json
   { "session_id": "...", "title": "...", "workspace_id": "...",
     "workspace_status": "created" | "reused",
     "behavior": "...", "status": "created",
     "worker_status": "started" | "idle", "auto_started": true | false }
   ```

### 8.2 `try_create_worksession`（UI session 唯一暴露的入口）

UI session LLM 看到的 args 只有一个：

```rust
try_create_worksession { reason: String }
```

实现路径：UI session 当前 turn 内 fork 出一个 sub-LLMContext，让 sub-context 基于
"最近聊天 + 现有 worksession 列表 + reason" 自由决定，最终通过 `create_worksession`
落地。`create_worksession` 的返回值即 `try_create_worksession` 的 tool result。

**Sub-context 的产出义务**（即调用 `create_worksession` 时要填齐的字段）：
- `title` —— 由 sub-LLM 概括一个简短标题（会出现在未来的 worksession list 里）
- `objective` —— 由 sub-LLM 从聊天记录＋reason 综合提炼
- `workspace_id` —— sub-LLM 决定：复用某个已有 workspace（填 id）还是新建（留空）。
  决策依据是 sub-context 注入的 worksession 列表 + UI session 当前绑定的 workspace
- `behavior` —— 一般留空走 `default_work_behavior`；只在 sub-LLM 明确判断需要特定 behavior 时填
- `reason_message: Vec<String>` —— sub-LLM 从最近聊天里**挑选**出真正促成本次创建的若干条
  原始 user 消息（保持原文，不要 LLM 改写），按时序传入。这是 worksession 起源凭据，
  会出现在新 worksession 的 `readme.md` 起源段

**Sub-context 构造（fork mode）**：

- `switch_mode = "fork"` — clone UI session 当前 `LLMContext` 快照，子上下文结束时丢弃其
  快照（不污染 UI session 的 step history）。这是 §3 中 Fork 模式的首个真实用例，
  落地它会同时关闭 §9.4 残项里 "Fork 真实实现" 一项。
- **系统提示词（fork 后重新渲染）**：
  - 标准的 role.md / self.md / users/*.md
  - **"现有 work session 列表"片段**：按 `updated_at_ms` desc 取，硬上限
    `MAX_WORKSESSION_LIST = 64`，每行 `<session_id> | <one_line_status> | <updated_at>`；
    数据源 = `AgentSession.meta` 扫盘
- **第一条 user message**（不属于 UI 主历史；fork 后注入）：
  - 最近 `MAX_FORWARDED_HISTORY = 32` 条 UI session 聊天记录的精简渲染（message 维度，
    跳过纯 tool_result）
  - 调用方传入的 `reason`
- **工具 whitelist** = UI session whitelist − { `try_create_worksession`, `forward_msg` }
  ＋ { `create_worksession` }。即"和 UI session 基本同构，去掉两个特殊工具，加上
  实际落地的工具"
- **`max_rounds`** 用独立常量 `WORKSESSION_PICK_MAX_ROUNDS = 8`，避免 sub-context 无限探索
- `output_spec` 用 `Text`——sub-context 最终必须通过工具调用落地，不需要 schema 化输出

**结果回传**：

- sub-context 成功（即调过 `create_worksession`）→ 把 `create_worksession` 的返回
  JSON 原样作为 `try_create_worksession` 的 `Observation::Ok` 返回给 UI session
- sub-context 失败（budget / error / 终止时未调过 `create_worksession`）→
  `Observation::Error { message }`，body 携带 `{ outcome, sub_trace_id, reason }`，
  让 UI session 下一轮 LLM 自己判断重试还是放弃（错误信息走标准 tool result 喂回，
  不抛 fatal）
- sub-context 的 step 历史**不**回写到 UI session（fork 语义保证）；sub snapshot
  在 fork 结束时清理

### 8.3 工程依赖

- §9.7 `local_workspace`：`create_or_open` / `set_current_session` 已就位；新建路径
  需要补一个 workspace_id mint 函数
- §9.4 残项中的 `Fork` switch_mode 真实实现 — 这两个工具一起拉通
- **不**依赖 `contact_mgr` / `task_mgr`

### 8.4 `forward_msg`（UI ↔ WorkSession 进程内路由）

UI session 唯一能直接把消息推到一个 worksession 的工具。典型场景：worksession 在
工作中向用户发了一个确认问题，用户在 UI 回复后，UI session 的 LLM 决定把这条用户
回复 forward 回原 worksession 让它继续推进。

```rust
forward_msg { target_worksession_id: String }
```

**"被转发的消息" = 触发本轮 UI session 推理的最近 user 消息**：即本轮 worker 从
`pending_inputs` 取走的 `PendingInput::Msg`，多条时取最新的一条。worker 在 turn
入口要把这个句柄存到本轮 tool context 里供 `forward_msg` impl 取用。

**路由实现**：
1. 校验 target：存在 / `kind == Work` / `status != Ended`，任一不满足返回
   `Observation::Error { message }`，下一轮 LLM 自我纠正（不抛 fatal）
2. 构造 `PendingInput::Msg`：
   - `record_id` 用合成 namespace `forward:<src_session_id>:<seq>`，**不会**进入
     `ack_msg_record` 路径（msg-center 不参与本次路由）
   - `from` = UI session 的 owner DID（保持来源可追溯）
   - `text` = 原 user 消息内容
3. `target_session.enqueue_pending(input)`：落盘成功后即返回
   `Observation::Ok { forwarded: true, target_session_id, record_id }`
4. target worksession 的 worker 会在 Idle / WaitingInput 状态下自然消费这条 pending

**不做的事**：
- 不调 `msg_center.post_send`（这是进程内路由，跨 tunnel 的转发是未来另一个工具
  的事，本工具不涉及）
- 不复制原 msg-center `record_id` / `message_id`（避免和真实记录冲突）
- 不支持 cross-agent（只能 forward 到本 agent 的 worksession）

---

## 9. 重构 checklist（给 CodeAgent）

### 当前进度（2026-05-14，第 3 轮更新 — 4 种 LLMContext 切换模式落地）

**本批次新增完成（[llm_context_helper.rs](src/frame/opendan/src/llm_context_helper.rs) 设计 + §9.4 switch_mode 真实化）：**

- **Phase 1 — helper 原语层**：新建 [llm_context_helper.rs](src/frame/opendan/src/llm_context_helper.rs)
  - `RequestOverrides`：纯数据覆盖结构（system_messages / tool_policy / objective / trace `Option<Option<String>>` / model_policy / budget / human_policy / error_policy / output / reset_rounds / reset_errors / forbid_next_behavior）
  - `apply_overrides_to_snapshot(snap, ov)`：纯 data 函数，**关键 invariant**——同步修改 `request.input` 和 `state.accumulated` 的前导 System 段（剥头部连续 System → 塞新 system → 后面非 System 部分保留）
  - `rebuild_with_inherit(base_snap, ov, deps)`：base_snap 的 `state.pending_tool_calls` 非空时返回 `SnapshotCorrupted`；否则 apply overrides → `LLMContext::resume(snap, ResumeFromMidRun, deps)`
  - `build_fresh(req, deps)`：`LLMContext::new` 的薄封装
  - 8 项单测覆盖 system 段同步 / reset_rounds 行为 / reset_errors 行为 / trace 三态（set/clear/keep）
- **Phase 2 — Switch（normal）模式真接通**：
  - `run_one_turn` / PendingTool resume 处的 `ctx.run()` 之后捕获 `ctx.snapshot()`——这是 **包含 final assistant message** 的完整后态（`Outcome::Done` 不携带 snapshot，但 ctx 仍活着）
  - `handle_outcome` 签名加 `final_snapshot: LLMContextSnapshot`
  - `Done + next_behavior(非 END)` 不再 discard，调 `switch_behavior(next, final_snapshot)`；后者按 `SwitchMode::Normal` 用 `apply_overrides_to_snapshot` 重建 → `persist_snapshot`；下一轮 `build_or_resume` 在新 system prompt + 完整历史下续跑
  - `apply_switch_normal`：按设计旋钮 `reset_rounds = false`（继承父预算）、`reset_errors = false`（防 LLM 切 behavior 绕过错误上限）
- **Phase 3 — Independent 模式真接通**：
  - `ProcessFrame { entry, current }` 结构 + `SessionMeta` 加 `process_entry: String` / `process_stack: Vec<ProcessFrame>`，都 `#[serde(default)]`；restore 路径 backfill 老 JSON（`process_entry == ""` → 用 `current_behavior`）
  - `behavior_snap_path(name)`——按 `local_workspace` 同款 `..` / `/` / `\\` / 空 id 防护
  - `persist_snapshot_to(path, snap)` / `try_load_snapshot_from(path)`——参数化版本；原 `persist_snapshot` / `try_load_snapshot` 改 delegate
  - `fresh_request_for(cfg)`——"用 behavior cfg 渲染全新 `LLMContextRequest`" 抽出共用
  - `apply_switch_independent`：父 `final_snapshot` 写到 `.meta/behavior_<父entry>.snap`；子 process snapshot（存量 → load + `reset_rounds/reset_errors` overrides；首次 → `LLMContextState::from_request` 新建）写到 `state.snap`；push 父 `ProcessFrame { entry, current }`；更新 `process_entry` + `current_behavior`
  - `handle_process_end(final_snapshot)`：栈空 → 顶层 process 真结束（`discard_snapshot + NextAction::End`）；栈非空 → 存子 final_snapshot 到 `.meta/behavior_<子entry>.snap`、装载父 snapshot 到 `state.snap`、注入 `[independent process \`X\` ended]` 系统消息到父 `pending_inputs`、`NextAction::Idle`
  - `handle_outcome::Done` 自然 Done（无 next_behavior）路径：栈非空 → `persist_snapshot(final_snapshot)` 保留子 process stream；栈空 → 保持原 discard（不破坏 UI session 多轮行为）
  - **关键不变量**：`state.snap` 始终镜像栈顶 process；`.meta/behavior_<entry>.snap` 仅在该 process 被挂起时存在；`current_behavior`（栈顶 process 内 normal switch 位置）与 `process_entry`（栈顶入口）只在 process 内做过 normal switch 时发散
- **Phase 4 — Fork 模式落地（session 私有原语，不暴露给 agent tool）**：
  - `AgentSession.fork_stack: Arc<Mutex<Vec<String>>>`——in-memory，每帧 = 父 trace id；不持久化（mid-fork 崩溃丢 sub-ctx，父从 on-disk snapshot 恢复，可接受）
  - `AgentSession::fork_and_run(overrides, sub_behavior_name) -> Result<ContextOutput>`：
    - 从 `try_load_snapshot()` 拿父 snapshot（TurnHook 在当前 inference 之前已写盘）
    - `sub_behavior_name` 加载子 cfg 做 deps（共享 parser/renderer / approval list / one_line_status sink）
    - `rebuild_with_inherit(parent_snap, overrides, deps)` 造子 ctx → `run()` → 提取 `ContextOutput`
    - 子 ctx suspended outcome（WaitInput / PendingTool / ContextLimitReached）映射为错误（fork 无 resume 路径）
    - inner helper `run_fork_sub` 保证 fork_stack push/pop 平衡
  - `fork_depth()` 公开访问器（async，共享同 mutex）
  - `AIAgent::get_session(id) -> Option<Arc<AgentSession>>` 公开访问器，session-aware 工具用它从 `Weak<AIAgent>` + `source_session_id` 拿到 `Arc<AgentSession>` 句柄
  - `TryCreateWorksessionTool`（`try_create_worksession`）：UI session 工具，构造时持 `Weak<AIAgent>` + `source_session_id`；`execute()` 内 `agent.get_session(id) → session.fork_and_run(overrides, parent_behavior)`，子 ctx 的 `ContextOutput::Json` 透传 / `Text` 包成 `{decision_text}`；sub-prompt 当前是最小指令（含 reason），未来补全 worksession list / parent recent history 注入
  - `register_worksession_tools` 加注册 `TryCreateWorksessionTool`
- **设计文档** [llm_context_helper.rs](src/frame/opendan/src/llm_context_helper.rs) 完整描述 3 种模式（switch / fork / independent）的语义、helper 2 个函数原语、Session-level 状态结构、旋钮倾向值、需要 waist 配合的未决项（`forbid_next_behavior` flag）
- 工程脚手架：`cargo test -p opendan --lib` **57/57 全绿**（+9：helper 8 个 + Session 旧 JSON backfill 1 个）

**架构观察**：3 个 switch_mode 在 helper 视角只需 2 个原语函数。差异完全收口在 session 端的 `apply_switch_normal` / `apply_switch_independent` / `handle_process_end` / `fork_and_run` 调用层。`switch_mode=Fork` 不再从 `next_behavior` 走 ——fork 是 session 内部原语，触发口在 session-aware 工具的 `call()` 实现里（典型：`try_create_worksession`），LLM 视角下看到的是一次普通 tool 调用。

### 当前进度（2026-05-14，下半轮更新）

**本批次新增完成：**
- §9.4 `PendingTool` 真接通：
  - `SessionMeta.pending_task_calls: Vec<PendingTaskCall>` 持久化字段（call_id ↔ task_id ↔ event_pattern 三元映射）
  - `handle_outcome::PendingTool` 现在调 `TaskDispatch::dispatch_async_tool` 创建 task_mgr 任务、`subscribe_event("/task_mgr/<task_id>")` 加订阅并持久化 mapping，返回 `NextAction::WaitForTool`
  - 新增 `persist_snapshot()`（tmp+rename 原子写）确保 PendingTool snapshot（含 `pending_tool_calls`）落盘，TurnHook 的 pre-inference 写入只覆盖 happy path
  - worker loop 重写：把 `pending_inputs` 里命中 `pending_task_calls` event_pattern 的 Event 单独成桶 → 用 `observation_from_task_event` 翻译 `to_status=Completed/Failed/Canceled` → 凑齐 snapshot.pending_tool_calls 后调 `LLMContext::resume(snap, ResumeFill::ToolResults{...})` 续跑；命中不全则保留 pending、wait
  - 续跑后自动 `clear_pending_task_calls()` + `unsubscribe_event(pattern)`，下一轮 PendingTool 干净起跑
  - `AgentSession.event_pump: Option<Arc<SessionEventPump>>` 字段让 `subscribe_event` / `unsubscribe_event` 立即推回 pump，agent 层不再需要中转 `refresh_session_subscriptions`（worker 里调 subscribe 直接生效）
  - 单测覆盖 `observation_from_task_event` 三种终态分支
- §9.4 PendingTaskCall + SessionMeta 字段（title / objective / bootstrap_done）round-trip 测试就位（48/48 全绿）
- §9.6 残项 from_name enrichment：`PumpConfig.contact_lookup: Option<Arc<ContactLookup>>` + `deliver_record` 在 record.from_name 缺失时调 `lookup.from_name(did)`；`Inbound::Msg` / `PendingInput::Msg` 新增 `from_name: Option<String>` 字段（持久化、round-trip 覆盖）；`AIAgent::spawn_msg_center_pump` 自动构造 ContactLookup 并喂给 pump
- §8.1 / §8.4 worksession 控制工具（`worksession_tools.rs`）：
  - `CreateWorksessionTool`（`create_worksession`）/ `ForwardMsgTool`（`forward_msg`）都是 `TypedTool` 实现，持 `Weak<AIAgent>` 防止 Arc 环
  - `ensure_session_inner` 在 `build_session_tools` 之后调 `register_worksession_tools(&tools, Arc::downgrade(&self), &session_id)` 注册到每个 session 的 manager（实际 LLM 是否能看到由 behavior whitelist 控制）
  - `AIAgent::create_work_session(params)`：workspace 解析（reuse/create）→ mint `ws-<uuid12>` session_id → 写 `readme.md`（title / objective / origin / reason_message）→ 调 `ensure_session_inner` 走标准创建路径 → `session.wake()` 触发 bootstrap turn
  - `AIAgent::forward_message(target, source, text)`：校验 target 是 Work 且未 Ended → `enqueue_pending(PendingInput::Msg { record_id: "forward:<src>:<uuid>", ... })`
  - Work session bootstrap：`SessionMeta.bootstrap_done` 标志 + worker loop 在「空 pending + 未 bootstrap + 有 objective」时自动跑首轮，无需外部消息（与 §8.1 step 4 对齐）
  - `render_system_messages` 现在把 `objective` / `title` 作为独立 `## Objective: <title>` 段插到 readme 前

### 当前进度（2026-05-14）

**已完成：**
- §9.1 `llm_context::xml_behavior` + `step_record`：`XmlBehaviorParser` / `XmlStepRenderer` 落地，27 项单测覆盖容错/多 action/压缩/交替等场景。
- §9.2 `opendan::ai_runtime`：5 个 deps 适配器全部实现。
  - `AiccLlmClient` / `OpendanToolAdapter` / `AgentPolicy` / `OpenDanWorklogSink` / `SessionSnapshotHook`
  - `AgentRuntime { aicc, worklog, msg_center, kevent_client, task_mgr }` —— 后三个为 `Option`，用 `with_msg_center` / `with_kevent_client` / `with_task_mgr` builder 注入；CLI 与单测可不连这三条边界
  - `SessionDepsInput { parser_renderer, ... }` + `build_session_deps()` 入口
  - `AgentPolicy` 做两道闸：approval list 与 whitelist 防御性二次校验
- §9.3 `opendan::behavior_cfg` + `opendan::agent_config`：
  - `BehaviorCfg` TOML 解析、`SwitchMode` / `BehaviorMode` / `BehaviorOutput` 翻译到 waist `ToolPolicy` / `HumanPolicy` / `ErrorPolicy` / `BudgetSpec` / `OutputSpec` / `ModelPolicy`，`build_parser_and_renderer()` 装 `XmlBehaviorParser` + `XmlStepRenderer`
  - `AgentConfig::open` 容忍 agent.toml 缺失；`builtin_ui_default()` 兜底；`list_behavior_names()` 扫盘
- §9.4 `opendan::agent_session`（**已升级为状态管理核心**）：
  - `AgentSession` + `AgentSessionBuild { existing_meta }` + `SessionInput { Wakeup, Cancel }` / `SessionReply` / `SessionMeta` / `SessionStatus`
  - **`PendingInput { Msg { record_id, from, from_did, tunnel_did, text }, Event { event_id, data } }` + `SessionMeta.pending_inputs: Vec<PendingInput>`** —— 持久化进 `.meta/session.json`，`#[serde(default)]` 兼容老格式；新增字段 `peer_did` / `peer_tunnel_did` / `event_subscriptions: Vec<EventSubscription>` / `workspace_id` 也全部持久化并能 round-trip 老 JSON
  - **`enqueue_pending(input)`**：dedup（按 `dedup_key`）+ push → `flush_meta()`（tmp + rename crash-consistent）→ Wakeup worker；落盘成功才返回 Ok（外部 ack 依赖这个返回值）
  - **`flush_meta()` 改成 `Result` 返回**，所有 caller 显式处理错误
  - **worker 改为从 `meta.pending_inputs` 消费**：snapshot pending → run_one_turn → 成功才 `discard_consumed` + flush_meta；失败保留以供下次 Wakeup / 重启重放（at-least-once）；`SessionInput` 现在是纯信号；Event pending 翻译成 `[environment event] {eventid} {json}` 与 Msg 文本同轮喂入
  - `build_or_resume`：优先尝试 `state.snap` resume；HumanInput / ResumeFromMidRun fill 已通；`AgentSessionBuild::existing_meta` 让 restore 路径保留 pending_inputs / peer / 订阅 / workspace_id
  - `handle_outcome` 覆盖 Done / WaitInput / PendingTool（warn）/ Budget / Error / ContextLimit；Done 路径会调 `post_outbound_text` 走 `msg_center.post_send` 把回复发回 peer
  - `switch_behavior`（Normal-only；Fork / Independent warn 后按 Normal 处理）
  - 新增 API：`subscribe_event` / `unsubscribe_event` / `subscription_patterns` / `set_workspace` / `workspace_id` / `update_peer`（私有）/ `post_outbound_text`（私有）
- §9.5 `opendan::agent_bash`：`build_session_tools(workspace, session_dir)` 注册 `exec_bash` + read/write/edit/glob/grep；`SessionBinLayout` 持有 4 层 bin 路径（System / Runtime / Agent / Session），目前 overlay 仅落 Session 层（upstream `BinOverlayConfig` 是单 `bin_dir`，4 层合成见上面"工程顺序 #4"）。**2026-05-14 新增 `TmuxBashRunner`**：`exec_bash` 不再用一次性 `/bin/bash -c`，每个 AgentSession 独占一个 detached tmux session（`od_<sanitized_session_id>`），每条 LLM 命令通过 wrapper 脚本 + run-id marker 在该 pane 里执行；用 `tee` 同时写 stdout/stderr 到 log 文件 + pane scrollback，操作员可 `tmux attach -t od_<sid>` 实时审计 AI 工作记录（pane 里能看到人类可读的 `# exec_bash[<run_id>] <command>` banner，不是不透明的 wrapper 路径）；超时走 `send-keys C-c`；session 数 ≥16 时按 24h idle GC。`exit N` 安全：用户命令包在 `( ... )` 子 shell 里，不会杀掉 wrapper 自身
- §9.6 `opendan::agent` + `opendan::msg_center_pump` + `opendan::session_event_pump`（**msg-center / kevent / outbound 全部接入**）：
  - `AIAgent::open(root, runtime)` 加载 `AgentConfig`、`AIAgent::run()` 驱动 dispatch loop（`tokio::select` { inbox, shutdown }）
  - **`Inbound { Msg { record_id, from, from_did, tunnel_did, session_id, text }, Event { event_id, target_session_id, data } }`** —— `from_did` / `tunnel_did` 用于 outbound 回送，`target_session_id` 由 session_event_pump 填充
  - **`msg_center_pump`**：用 `KEventClient` 订阅 `/msg_center/{owner}/box/**` 系列模式，`pull_event(1s)` hit / miss / ReaderClosed 全部走同一条 `msg_center.get_next` 路径（kevent 是加速通道、不是真理来源——超时落到 sweep all inbox boxes）；翻译成 `Inbound::Msg`（含 sender DID 全形 + `route.tunnel_did`）后 send 到 `inbox_tx`，**自己不做 ack**
  - **`session_event_pump`**（新模块）：单 `EventReader` 聚合所有 session 的 `event_subscriptions` 模式；`set_session_subscriptions` / `remove_session` + `refresh` Notify 触发 reader 重建；`pull_event` 命中后调 `match_event_patterns` 对每个匹配 session fan-out `Inbound::Event { target_session_id: Some(sid), ... }`；shutdown / `ReaderClosed` / 空订阅状态全部分支处理
  - **`dispatch_inbound`**：Msg 路由到 session → `enqueue_pending(...)` 落盘完成 → `ack_msg_record(record_id)` 调 `msg_center.update_record_state(Readed)`；Event 按 `target_session_id` 投到匹配 session
  - `restore_active_sessions()` 从盘上的 `.meta/session.json` 恢复非 Ended，**通过 `AgentSessionBuild::existing_meta` 把订阅 / pending / peer / workspace_id 一并还原**；重启自动重放 `pending_inputs` 里残留的输入，并把订阅推回 event_pump
  - `AIAgent::refresh_session_subscriptions(sid)` 公开接口，工具实现修改完 `AgentSession::subscribe_event` 后调它通知 event_pump
  - **outbound 回送**：`AgentSession::post_outbound_text` 用 `agent_did` 当 sender、`peer_did` 当 to、`peer_tunnel_did` 当 `preferred_tunnel`，组装 `MsgObject { thread.{topic,correlation_id} = session_id, meta.session_id = session_id }` 后调 `msg_center.post_send`；失败仅 warn，本地 reply 不受影响
  - reply 收集任务：每个 session 起一个 logger，把 AssistantText / Error / PromptToHuman / Ended 写日志（outbound 已经在 session 内部送出，logger 仅为可观测性）
  - `main.rs`：bootstrap 拉 `MsgCenterClient` + 构造 `KEventClient::new_full(OPENDAN_SERVICE_NAME, None)` + `TaskManagerClient`（任一不可用时 warn 降级），SIGINT 走 `shutdown()` graceful 退出
  - shutdown 协调：`pump_shutdown: Arc<Notify>` 同时让 msg_center_pump + session_event_pump 关掉各自的 EventReader
- §9.7 `opendan::local_workspace`（**已重写为 AgentSession-owned 绑定**）：
  - `WorkspaceRecord { workspace_id, name, created_by_session, current_session, created_at_ms, updated_at_ms, status }` + `WorkspaceStatus { Ready, Archived, Error }`
  - `LocalWorkspaceManager` 无内存状态（只持 `workspaces_root: PathBuf`），`create_or_open` / `load_record` / `save_record` / `set_current_session` / `list` / `archive` 全部直读直写盘；tmp + rename crash-consistent
  - `validate_workspace_id` 拒 `..` / `/` / 空 id（防 path traversal）
  - 旧版的全局 `session_bindings: HashMap` 删除——session 绑定由 `AgentSession.meta.workspace_id` 持有（持久化进 session.json），workspace 记录里的 `current_session` 仅作冲突检测 hint
  - `AIAgent` 持 `LocalWorkspaceManager`，`ensure_session_inner` 现在调 `create_or_open` 维护工作区记录 + `set_current_session` 双向绑定（session-side 是真理源），重启从 `existing_meta.workspace_id` 复用工作区；`workspaces()` accessor 给后续 `try_create_worksession` 工具
- §9.8 `opendan::contact` + `opendan::task_dispatch`（**task_mgr / contact_mgr 骨架就位，等具体功能接入**）：
  - `ContactLookup { msg_center, owner }` —— `from_name(did)` 走 `msg_center.get_contact`，TTL 分级缓存（hit 5min / miss 1min / 错误不缓存）；`invalidate()` 手动清缓存
  - `TaskDispatch { client: Arc<TaskManagerClient>, user_id, app_id }` —— `dispatch_async_tool(session_id, tool_name, payload)` 创建 `TASK_TYPE_OPENDAN_TOOL = "opendan.async_tool"` 任务并返回 `DispatchedTask { task_id, task }`；`mark_task_completed(task_id, success)` 给 PendingTool 收尾用
  - `AgentRuntime.task_mgr` 字段 + `main.rs` bootstrap（不可用时 warn 降级）；ContactLookup 当前由调用方按需 `new(msg_center, owner)` 构造，未塞进 AgentRuntime 是因为 owner 会随 agent_did 变化
- 工程脚手架：`cargo test -p opendan --lib` **44/44 全绿**（新增覆盖 outbound 字段 round-trip、event_subscriptions / workspace_id 字段持久化、session_event_pump 路由 + dedup、local_workspace CRUD / 校验、contact TTL、task_dispatch tag 常量等）

**§9.2 残项（4 层 bin overlay 真接）已落地（2026-05-14 第 4 轮，PATH 拼接 + Session 渲染模型）：**

- **(a) upstream `agent_tool::BinOverlayConfig` 扩成多层**：`bin_dir: Option<PathBuf>` → `layers: Vec<PathBuf>`，新增 `BinOverlayConfig::layered([...])` 构造器；`prepare_overlay_env` 反向遍历 layers 用同一个 `prepend_path_entry` 拼，`layers[0]` 落在最前。`local(path)` 向后兼容（包成 1-layer），`bin_overlay_shadows_system_path` 现有测试无须改；新增 `overlay_env_stacks_multiple_layers_in_priority_order` 覆盖 4-layer 顺序。
- **(b) opendan 路径切换 + agent_id 透传**：
  - 新增 [paths.rs](src/frame/opendan/src/paths.rs)：`buckyos_root()` / `buckyos_tools_root()` / `system_bin_dir()` / `runtime_bin_dir()` / `session_exec_bin_dir(agent_id, session_id)` 单一来源；`BUCKYOS_ROOT` env override + per-OS dev fallback（Linux `/opt/buckyos` / macOS `$HOME/.buckyos` / 其它 `/tmp/buckyos`），`sanitize_path_segment` 把任意 id 收敛到 `[A-Za-z0-9_-]`。
  - `SessionBinLayout` 重写：`compute(agent_id, session_id, agent_root)` 同时算 System / Runtime / Agent / Session 四层；`ensure_dirs()` 只 mkdir 后三层（System 由 BuckyOS 镜像预置）；`to_overlay()` 输出 `BinOverlayConfig::layered([session, agent, runtime, system])` 直接对应 §2 优先级。
  - `AgentLayout` 加 `tool_plans_dir = <agent_root>/tool_plans/` + `tool_plan_path(name)`；`AIAgent::agent_id()` accessor（优先 `agent_did`，回落 `agent_name`，过 sanitize），`build_session_tools` 签名换成 `SessionToolsBuild { workspace_root, session_dir, agent_root, agent_id, session_id, bin_renderer }` 结构体。
- **(c) Session Exec Bin 渲染器**：新增 [tool_plan.rs](src/frame/opendan/src/tool_plan.rs)
  - `ToolPlanToml` + `PlanMode { Deny, Allow }` 解析；`ResolvedToolPlan::resolve(plan_name, plan, universe)` 把 plan + bin 宇宙翻成最终 tombstone 列表（deny 模式取列表与宇宙交集；allow 模式取宇宙差集）。`scan_bin_universe([dirs])` 扫 System / Runtime / Agent 三层（顶层 + 一层子目录，按 §1.5 hot-path 约定）。
  - `SessionBinRenderer { session_bin, agent_tools, plan_name, resolved, last_sync_mtime_ns, linked }`：
    - `render_initial(session_dir)`：mkdir session_bin → force `apply_snapshot`（hard-link agent_tools 顶层 + 一层子目录的可执行文件到 session_bin，跨 fs / 非 Unix 退 `fs::copy`） → 写 tombstone stub 文件（shebang，stderr 双行 JSON + 人类可读，`exit 127`） → 把 `ResolvedToolPlan` 序列化到 `<session_dir>/tool_plan.resolved.toml` 供操作员审计。
    - `maybe_resync()`：`snapshot_agent_tools` 算 max_mtime_ns；不大于上次同步 + 入口非空就直接跳过；否则 `apply_snapshot` re-link（不在 snapshot 的旧 link 自动 rm）+ 重新落 tombstone。
    - tombstone 永远 last-writer-wins：`apply_snapshot` 主动跳过和 tombstone 同名的 agent_tools 文件，再交给 `write_tombstones` 写脚本。
  - `TmuxBashRunner::with_bin_renderer(renderer)` builder：`run()` 起手调 `renderer.maybe_resync()`，失败仅 warn（不阻 exec_bash）。
  - `BehaviorCfg` 新增可选字段 `tool_plan: String`（默认空 = 无 tombstone）。
  - `AIAgent::build_session_bin_renderer(agent_id, session_id, behavior_name)`：load 行为 cfg 取 `tool_plan` 名 → load `<agent_root>/tool_plans/<name>.toml` → 扫宇宙 → resolve → 包成 `Arc<SessionBinRenderer>`；行为 cfg / 计划文件缺失全部 warn + 空计划兜底（仍做 Agent tools 同步）。
  - 集成测覆盖：`overlay_env_stacks_multiple_layers_in_priority_order`（agent_tool 75/75 全绿）、`session_bin_layout_overlay_has_four_layers`、`plan_deny_mode_picks_only_universe_intersect`、`plan_allow_mode_tombstones_everything_else`、`write_tombstone_creates_executable_script`、`render_initial_links_agent_tools_and_writes_resolved_plan`、`paths::buckyos_root_layout_honors_env_override`（opendan 69/69 全绿）。
- **已知尾巴**：
  - behavior 切换时（normal / independent / fork）目前不触发 tool plan 重算——`SessionBinRenderer` 是 session 创建时一次性绑定的；可在 `agent_session::switch_behavior` 里加一个回调让 `AIAgent` 重新算 `Arc<SessionBinRenderer>` 并 hot-swap 进 `TmuxBashRunner`。等真的有 behavior 间 tool plan 差异的用例再回看。
  - System Bin 在 `ensure_dirs` 不 mkdir（生产容器里 `/opt/buckyos/tools/store/` 由镜像预置）；开发机 BUCKYOS_ROOT 指到任意目录时也照样 mkdir 不到，PATH 里挂个不存在的目录无害（shell 跳过）。

**仍未完成：**
- ~~§9.2 残项~~（已落地，见上）。设计要点保留如下供后续 behavior-switch tool plan 重算 / Runtime Bin 真接（ExtTool Volume）/ Crafter 镜像接入参考：
  - **路径契约**（来自 [paios 容器需求.md §9](paios容器需求.md)）：
    - System Bin：`<buckyos_root>/tools/store/` — rx，全 App 共享
    - Runtime Bin：`<buckyos_root>/tools/bin/` — rx，App-scoped 渲染（第一版可空目录占位，等 ExtTool Volume 接入）
    - Agent Bin：Agent Root 内的 `tools/`（Instance Volume，rwx）
    - Session Exec Bin：`<buckyos_root>/tools/<agent_id>/<session_id>/` — rwx，session 启动时渲染
  - **三件事要做**：
    1. **upstream `BinOverlayConfig` 改成多层 PATH 列表**（最简实现：`Vec<PathBuf>` 按顺序前缀拼 PATH；`prepare_overlay_env` 顺序遍历）。无需引入每层权限属性——权限由 OS 文件系统层做。
    2. **Session Exec Bin 渲染**（在 opendan，启动器视角，session 创建时跑一次）：
       - 把 Agent Root FS `/tools/` 的内容拷贝（或硬链接 + COW）到 Session Exec Bin。
       - 读 session 自带的 plan 文件，按规则在 Session Exec Bin 写 stub wrapper 屏蔽下层同名工具（墓碑效应）。
    3. **`SessionBinLayout` 路径计算** 改成基于 `<buckyos_root>` + `agent_id` + `session_id`。
  - **接口前提**：
    - `agent_id` 是 `AIAgent` 的属性（类比 paios 里的 app_id），`AIAgent::ensure_session_inner` 调 `build_session_tools` 时直接传即可。
    - `<buckyos_root>` 先写死一个 `fn buckyos_tools_root() -> PathBuf` helper（默认 `/opt/buckyos/tools`，可被 `BUCKYOS_ROOT` 环境变量覆盖给开发机用）；等 BuckyOS 那边整理出统一的路径文档后再回头切到正式 API。
  - **已决设计要点**（2026-05-14 第 4 轮）：
    1. **plan 文件格式 / 位置 / 引用方式**：
       - 格式：TOML，与 behavior config / Cargo 一致，不引新格式。
       - 位置：`<agent_root>/tool_plans/<plan_name>.toml`——Agent 层（不是 session），plan 是 owner/operator 的策略而非 session 临时决定；多个 plan 可共存。
       - 引用：Behavior config 里新增可选字段 `tool_plan = "<plan_name>"`（缺省 = 全部可见）。不同 behavior 可挂不同策略（UI 模式宽松，code-exec worker 严格）。
       - schema：
         ```toml
         mode = "deny"     # "deny" | "allow"，默认 "deny"
         [[deny]]
         name   = "rm"
         reason = "use trash-cli instead"
         # mode = "allow" 时改用 [[allow]]，未列的工具一律墓碑
         ```
       - 解析产物：session 启动时把"实际生效的合成策略"落到 `<session_dir>/tool_plan.resolved.toml`，跟 tmux pane scrollback 一起给操作员事后审计用。
    2. **Agent tools 拷贝时机**：**每次 `exec_bash` 起手做一次 mtime 同步检查**（跟 §1.5.6 现有约定一致：`exec_bash` 调用 head 做 mtime walk，有改动才 re-render Session Exec Bin）。
       - 动机：用户/operator 手工往 `<agent_root>/tools/` 拷新工具后，**不重启 session 也能在下一次 `exec_bash` 立即可用**。
       - 成本理由：两次 `exec_bash` 之间至少隔一次 LLM 推理（秒级以上），多一次 mtime walk（毫秒级）成本可忽略；不需要 inotify watch / 后台同步线程。
       - 不暴露显式 `reload_session_tools` 工具——同步是隐式的，LLM 不必感知。
       - 拷贝形式：Linux 容器内用 hard link（零空间成本 + 写时自然 COW，session 改了文件不污染 Agent 持久层）；跨 fs / 非 Linux 退回 `copy_if_changed`。
       - 拷贝范围：只拷可执行 + 顶层 + 一层子目录，与 §1.5 hot-path 约定一致。
    3. **墓碑 stub 形态**：shebang 脚本，stderr 双行（JSON + 人类可读），exit 127。
       ```sh
       #!/bin/sh
       # auto-generated by opendan tool plan renderer
       echo '{"blocked_by":"tool_plan","tool":"rm","reason":"use trash-cli instead","plan":"minimal_safe"}' >&2
       echo 'rm is blocked by tool plan: use trash-cli instead' >&2
       exit 127
       ```
       - exit 127 是 shell "command not found" 标准码，做错误判断的脚本不用改。
       - JSON 给 LLM 解析（它能从 `reason` 学到换什么工具），第二行给 tmux 审计的人。
    4. **跨平台开发机**：`buckyos_root()` helper 集中所有路径，单一 `BUCKYOS_ROOT` 环境变量 + per-OS dev fallback。
       ```rust
       pub fn buckyos_root() -> PathBuf {
           if let Ok(v) = std::env::var("BUCKYOS_ROOT") { return PathBuf::from(v); }
           #[cfg(target_os = "linux")]   { PathBuf::from("/opt/buckyos") }
           #[cfg(target_os = "macos")]   {
               std::env::var_os("HOME")
                   .map(|h| PathBuf::from(h).join(".buckyos"))
                   .unwrap_or_else(|| PathBuf::from("/tmp/buckyos"))
           }
           #[cfg(not(any(target_os = "linux", target_os = "macos")))]
           { PathBuf::from("/tmp/buckyos") }
       }
       pub fn buckyos_tools_root() -> PathBuf { buckyos_root().join("tools") }
       ```
       生产容器里始终 `export BUCKYOS_ROOT=/opt/buckyos`；开发机什么都不设也能跑。等 BuckyOS 路径文档落地后只改这一处。
    5. **Runtime Bin**：**空目录占位**——`<buckyos_root>/tools/bin/` 在 session 启动时 mkdir 一份，照样进 PATH。
       - 第一版没人往里写东西也无害；ExtTool Volume / Crafter 接入时只需往这层塞 symlink，无需改 overlay PATH 拼接逻辑。
       - 把"一处麻烦"留给未来的自己 = 反模式，避免。
  - **当前实现状态**：[agent_bash.rs](src/frame/opendan/src/agent_bash.rs) 的 `SessionBinLayout` 还是单层 `<session_dir>/bin`，`BinOverlayConfig` 还是 upstream 单 `bin_dir`，三件事都没动。tmux runner 已落地不影响 overlay 工程项。
- §9.4 残项（switch_mode 三模式 + fork 原语 2026-05-14 第 3 轮已落地；剩余尾巴 2026-05-14 第 5 轮三项全部落地）：
  - ~~`try_create_worksession` 子上下文 prompt 渲染补全~~ — 已落地（worksession list / 可用 workspace 列表 / parent recent history 三段都注入 sub-system；`forward_msg` 自动从 session 取 origin user 消息）
  - ~~**waist 真接 `forbid_next_behavior` flag**~~ — 已落地（[request.rs](src/frame/llm_context/src/request.rs) 加 `forbid_next_behavior: bool`，[behavior_loop run_behavior](src/frame/llm_context/src/context_loop.rs:624) 解析后 scrub `next_behavior`；`TryCreateWorksessionTool` 现在 `forbid_next_behavior: true`）
  - ~~`RequestOverrides` + `apply_overrides_to_snapshot` 下沉到 `llm_context` crate~~ — 已落地（[snapshot_overrides.rs](src/frame/llm_context/src/snapshot_overrides.rs)，opendan helper 退化为薄 `pub use` shim）
  - ~~`ContextLimitReached` 的消息层压缩 + `ResumeFill::RewrittenHistory` 续跑~~ — 已落地（`run_one_turn` 内置 MAX_COMPRESS_ROUNDS=3 的 re-entry loop；`compress_messages_for_context_limit` 保留前导 System + 末尾 12 条 + 单条 `[context compressed]` 合成 User 消息；持久化 rewritten snapshot 后 resume）
  - ~~"环境感知 message"~~ — 已落地（`compose_environment_message`：behavior name / session_id+title / workspace_id / one_line_status / unix_ms 五项，仅在 turn 有真实 human/event 输入时拼到 user message 前缀；auto-recall memory / event diff 仍是后续扩展点）
  - ~~`forward_msg` 自动抓取 "本轮 origin user 消息"~~ — 已落地（`AgentSession.current_origin_msg` 在 worker turn 入口由 worker 写入；`ForwardMsgArgs.message` 改为 `Option`，缺省走 `current_origin_user_message()`）
  - **independent 跨 process normal switch 的恢复路径**：当前 `process_stack` 帧记 `(entry, current)`，pop 时恢复 current；但如果某个 process 内做过 normal switch，存量 `behavior_<entry>.snap` 的 system 段是 switch 后的、`process_stack.current` 也是 switch 后的——这两边要不要"对齐到 entry 自身的 system" 是个语义抉择，等真实用例出来再回看

### 当前进度（2026-05-14，第 5 轮更新 — 剩余 3 项工程项全部落地）

**本批次新增完成（[snapshot_overrides.rs](src/frame/llm_context/src/snapshot_overrides.rs) / [agent_session.rs](src/frame/opendan/src/agent_session.rs) / [worksession_tools.rs](src/frame/opendan/src/worksession_tools.rs)）：**

- **Task 1 — waist `forbid_next_behavior` flag + `RequestOverrides` 下沉**：
  - [request.rs](src/frame/llm_context/src/request.rs) `LLMContextRequest` 加 `forbid_next_behavior: bool`（`#[serde(default)]` 老 JSON 兼容），所有 caller（opendan / agent_tool::local_llm_context / llm_context tests）回填字段
  - [behavior_loop run_behavior](src/frame/llm_context/src/context_loop.rs:624) 解析完 `StepRecord` 后看 flag，命中则 `new_step.next_behavior.take()` + `log::warn`，让 sub-ctx 走到自然 End；`assistant_text` 不动（LLM 思考过程保留）
  - 新文件 [snapshot_overrides.rs](src/frame/llm_context/src/snapshot_overrides.rs)：`RequestOverrides` / `apply_overrides_to_snapshot` / `rebuild_with_inherit` / `build_fresh` 从 opendan helper 整体搬过来，`apply_overrides_to_snapshot` 现在真把 `RequestOverrides.forbid_next_behavior` 写到 `snap.request.forbid_next_behavior`（sticky flag — 子 fork 嵌套自动继承）
  - opendan helper 退化为薄 `pub use` shim（保留设计文档），全 9 项单测（含新 `apply_overrides_forbid_next_behavior_sets_request_flag`）跟随类型搬到 llm_context
  - [worksession_tools.rs](src/frame/opendan/src/worksession_tools.rs) `TryCreateWorksessionTool` 现在 `forbid_next_behavior: true`
- **Task 2 — `try_create_worksession` sub-prompt 补全 + `forward_msg` origin msg 自动抓取**：
  - [agent_session.rs](src/frame/opendan/src/agent_session.rs) 新增 `SessionSummary` 结构（session_id / kind / title / objective / status / one_line_status / workspace_id / current_behavior）+ `AgentSession::summary()`；`AIAgent::list_session_summaries(exclude_id)` 公开聚合
  - `AgentSession.current_origin_msg: Arc<std::sync::Mutex<Option<String>>>` 字段；worker 在 turn 入口（具体是 turn_inputs 非空那一刻）从 `human_texts` 末尾抽最新非 `[environment event]` 项写进去；`AgentSession::current_origin_user_message()` 公开读
  - `AgentSession::try_load_snapshot_for_prompt()` 公开读 parent snapshot（resume 路径仍走私有 `try_load_snapshot`）
  - [worksession_tools.rs](src/frame/opendan/src/worksession_tools.rs) `render_sub_system_prompt` / `render_worksession_inventory`（过滤 Ended + Work 优先 + 64 条上限）/ `render_workspace_inventory`（按 updated_at_ms desc 排，32 条上限，空时给"留空 mint 新 ws"提示）/ `render_parent_recent_history`（过滤 system/tool，user/assistant 末尾 32 条，单条截 480 字符）四段拼起来，加 `parent_workspace_id` 偏好提示
  - `ForwardMsgArgs.message` 改 `Option<String>`：缺省自动走 `session.current_origin_user_message()`，缺也缺时返回明确错误（"the current turn appears to have been driven by an event / tool result"）
- **Task 3 — `ContextLimitReached` 重写续跑 + "环境感知 message"**：
  - `AgentSession::run_one_turn` 内置 ContextLimitReached re-entry loop（`MAX_COMPRESS_ROUNDS = 3`）：拦截 outcome → `compress_messages_for_context_limit(accumulated)` → 写盘新 snapshot → `LLMContext::resume(ResumeFill::RewrittenHistory)` → 继续 `ctx.run()`；3 轮还命中就翻译成 `Outcome::Error` 走标准 handler
  - `compress_messages_for_context_limit`：保留前导 System 段；丢中间，保末尾 `COMPRESS_KEEP_TAIL = 12` 条；插一条合成 User 消息描述 `dropped` 数；realign tail 跳过开头 Assistant 防 Assistant→Assistant 连发；alternation 测试通过
  - `handle_outcome::ContextLimitReached` 退化为"不应该到这"的 warn 兜底（re-entry loop 拦截掉了）
  - `AgentSession::compose_environment_message(behavior) -> Option<String>` 拼 behavior / session_id+title / workspace_id / recent activity / unix_ms 五段，前缀 `[environment]`；`meta.try_lock` 失败安全降级 None
  - `build_or_resume` 改为：先 `compose_human_text(human_texts)`，**有人类输入时**才拼 env body 进 user_message_text（mid-run resume / bootstrap 不污染 history）；`merge_env_and_human` 处理 Some/None 各组合
  - 单测覆盖 `compress_messages_preserves_short_history_verbatim` / `compress_messages_drops_middle_and_keeps_tail`（含 no-back-to-back-assistant 不变量）/ `merge_env_and_human_combines_both_with_env_first` / `merge_env_and_human_handles_missing_pieces`，加上 worksession_tools 的 8 项 prompt 渲染单测
- 工程脚手架：`cargo test -p llm_context --lib` **74/74 全绿**（+1 forbid flag 测试），`cargo test -p opendan --lib` **73/73 全绿**（+12：compressor / merge_env / 8 worksession_tools prompt 渲染 / origin msg auto-capture 间接覆盖）

**架构观察**：3 项尾巴各自封闭、互不交错。`forbid_next_behavior` 通过 sticky request flag 让嵌套 fork 自动继承，无需调用方逐层透传；`RequestOverrides` 下沉后 opendan 不再持有 waist-internal 类型的"私版"，等价于把 [llm_context_helper.rs](src/frame/opendan/src/llm_context_helper.rs) 提示的 Phase 5 全部交付。`ContextLimitReached` 用 re-entry loop 而不是新 NextAction，控制流不外泄；`compose_environment_message` 只在 turn 有真实输入时才注入也防止了"每次 wakeup 都写一条 env 进 history"的污染。下一步 follow-up 是把 env body 的 auto-recall memory / event diff 真接上（目前是占位骨架）。


### 工程顺序（剩余）

- 本轮完成（2026-05-14 第 5 轮）：上面三项历史排序号 #1 / #2 / #3 全部交付，工程顺序清单清空。
- 后续可选 follow-up：
  - 把 env body 的 auto-recall memory + event diff 真接上（目前 `compose_environment_message` 只有 behavior / session / workspace / one_line_status / clock 五段骨架）
  - independent 跨 process normal switch 的恢复路径语义抉择（见上面"残项"段最后一条）
  - behavior 切换时 `SessionBinRenderer` hot-swap（目前 session 启动绑定一次）
  - Runtime Bin 真接（ExtTool Volume / Crafter 接入）
  - 4 层 bin overlay 实施 — 已落地（2026-05-14 第 4 轮）。(a) 多层 `BinOverlayConfig`、(b) `paths::buckyos_root()` + `SessionBinLayout::compute(agent_id, session_id, agent_root)` + `SessionToolsBuild`、(c) `SessionBinRenderer`（hard-link Agent tools + mtime resync + tool plan tombstones + `tool_plan.resolved.toml` 审计）全部就位，agent_tool 75/75 + opendan 69/69 全绿。

每个阶段独立编译 + 跑 `cargo test`。当前 opendan 已可：
- 从 msg-center 拉 msg → `Inbound::Msg` → UI session → `enqueue_pending` 落盘 → ack `Readed` → worker 在合适状态下走 `exec_bash` + 读文件 → outcome `Done` 时把回复 `post_send` 回原 peer DID（用 record 上的 `route.tunnel_did` 当 `preferred_tunnel`）
- 进程崩 / 重启：未消费的 msg、peer 路由信息、kevent 订阅列表、workspace 绑定、`pending_task_calls`、process 调用栈（`process_entry` / `process_stack`）、各 process 的独立 snapshot 文件全部从 `.meta/` 还原（at-least-once）
- 任何 session 通过 `subscribe_event(pattern)` 加订阅 → 直接走 `event_pump.set_session_subscriptions` 立即生效 → `session_event_pump` 重建 reader → kevent 命中自动派发回该 session 的 `pending_inputs`
- LLM 触发 `PendingTool` outcome → 自动转 `task_mgr` 任务、订阅 `/task_mgr/<task_id>`、session 进入 `WaitingTool` → 完成事件回来后自动 `ResumeFill::ToolResults` 续跑
- LLM 调 `create_worksession { title, objective, ... }` → 新建 workspace（按需）+ 新建 Work session 目录 + `readme.md` + 自动 bootstrap 首轮推理（objective 渲染进 system prompt）
- LLM 调 `forward_msg { target_worksession_id, message }` → 进程内路由进 target 的 `pending_inputs`（合成 `record_id`，不走 msg-center）
- **LLM 在 Done 时声明 `<next_behavior>X</next_behavior>` → 按 X 的 `switch_mode` 触发不同切换**：
  - `Normal` → 同 process 内换 system prompt + tool whitelist，step record stream 全保留
  - `Independent` → push 父 process 帧到 `process_stack`，子 process 从 `.meta/behavior_<X>.snap` 续跑（或首次 `from_request` 起步），父帧静默挂起
  - `END` → 若栈空真结束 session；若栈非空 pop 父帧 + 注入 `[independent process \`X\` ended]` 系统消息唤醒父
- **LLM 调 `try_create_worksession { reason }` → session 内部 fork sub-ctx 跑 fork-decision 推理**：sub-ctx 只能调 `create_worksession`，跑到 Done 后 `ContextOutput` 作为 tool result 回填给父 LLM；父 ctx 视角下看到的就是一次普通 tool 调用，PendingTool / ToolResults 路径天然适用
- msg-center pump 自动用 `ContactLookup` 给缺 `from_name` 的 record 补显示名（hit 5min / miss 1min TTL），LLM 提示词看到的是人名而非裸 DID
