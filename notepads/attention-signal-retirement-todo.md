# Attention Signal 退役 TODO（Memory TD-20 收尾）

日期：2026-11-01　状态：已实施（T1–T6，见 §4）；全新 DV 跑 Jarvis 整理 Goal 一轮未做

## 0. 结论

`agent_tool/src/agent_attention_signal.rs`（2464 行，SQLite 管线 + Stage1 扫描 + 四个 `Discover*` TypedTool）及其在 `agent_tool_cli_dev` 里的 8 个 CLI 工具整体删除。它在设计上已被《Agent Memory 认知管理需求》的感知/认知模型取代（§1 把旧设计列为"只作参考"；TD-20 要求"只保留一条感知管线"），代码上已无运行时调用方：驱动它的两份 behavior `self_improve_signals.toml` / `self_improve_set_memory.toml` 在 Jarvis 默认整理 Goal 那次已删，rootfs 里没有任何 toml/prompt 再引用这些工具名。当时按 Jarvis Goal TODO T03 "本次不删"留下的，本 TODO 专门退役它。

TD-20 说的"仍有价值的部分"已经不需要搬代码：
- 泛指类型词过滤（`is_generic_object_word`）：需求 §4.5 正反例表已写"'超市'这类泛指类型词不作对象"，是提示词规则而不是代码校验，不并入。
- 合法性校验（`validate_signal` 系列）：校验对象是旧 signal 结构，新感知流的结构不同（`memory/perception.rs::check_source` 等已有自己的来源校验），不并入。
- 事后回扫：仍是需求 §11 待决项（M-11），与本次退役无关，不在此实现。

Notebook（TD-09）**不在本 TODO 范围**，它还有 `lib_opendan` 的 `cognition.notebook_append` 等活调用方，退役时另立 TODO。

## 1. 现状盘点（2026-11-01）

### 1.1 代码
| 位置 | 内容 | 处置 |
|---|---|---|
| `src/frame/agent_tool/src/agent_attention_signal.rs` | 全部 | 删文件 |
| `src/frame/agent_tool/src/lib.rs:22` | `pub mod agent_attention_signal;` | 删 |
| `src/frame/agent_tool/src/lib.rs:54-63` | `pub use agent_attention_signal::{...}` 整段 | 删 |
| `src/frame/agent_tool/Cargo.toml` | `rusqlite`、`chrono` | **保留**：`agent_notebook.rs` 还用 `rusqlite`；`chrono` 另有 dcrontab/xllm 等 5 个使用者 |
| `src/frame/agent_tool_cli_dev/src/lib.rs:24-28` | `use agent_tool::agent_attention_signal::{...}` | 删 |
| 同文件 `:44-52` | 从 `agent_tool::{}` 里引入的 `AgentAttentionSignalStore`、`AttentionSignalStoreConfig`、`AttentionSignalToolRuntime`、`DiscoverEventTool`、`DiscoverObjectObservationTool`、`DiscoverRelationshipTool`、`DiscoverSkillCoverageGapTool` | 删这 7 个名字 |
| 同文件 `:65-68` | 4 个 `TOOL_*_ATTENTION_*` 常量 | 删 |
| 同文件 `:69-94` | `TOOL_NAMES: [&str; 24]` | 去掉 8 个条目，改为 `[&str; 16]`（`handles_invocation_name` 据此分发，少了就不再被 `opendan` 二进制接管） |
| 同文件 `:106-107` | `ATTENTION_EXTRACTION_RUNTIME_REL_PATH` | 删 |
| 同文件 `:3750-4160` | 8 个 Args/Output 结构、8 个 `Cli*Tool` 结构与 `TypedTool` impl | 删 |
| 同文件 `:4172-4245` | `session_dir`（仅 attention 用）、`open_attention_store`、`attention_runtime_path`、`save/load/remove_attention_runtime` | 删；`validate_session_id_arg`（:4154，唯一调用方是 `CliBeginAttentionSignalExtractionTool`）一并删 |
| 同文件 `:4273-4309` | `build_cli_tool_manager` 里 8 次 `register_typed_tool` | 删 |
| 同文件 `:5202-5390` | 测试 `attention_stage1_cli_flow_discovers_and_completes`、`attention_stage2_cli_lists_and_marks_consumed` | 删 |
| `src/frame/lib_opendan/src/state/fs_client.rs:12` | 模块注释 `memory/ notebook/ attention_signals/ cognition (agent_tool)` | 去掉 `attention_signals/` |

删完后 `cargo check -p agent_tool -p agent_tool_cli_dev -p opendan -p lib_opendan`，再 `cargo test -p agent_tool -p agent_tool_cli_dev`；留意 cli_dev 里 `schemars`/`Arc`/`fs` 等 import 是否变成未使用。

### 1.2 数据
- `<agent_root>/attention_signals/`（SQLite 库 + 检查点 + extraction windows）与 `<agent_root>/sessions/<sid>/.runtime/attention_signal_extraction/current.json`。
- 处置：**不迁移、不自动删除**。按"DV 可全新安装、不写兼容"原则，不写读旧库的代码；旧目录留在磁盘上无害，DV 重装自然消失。Memory 多 Session 模拟 TODO 第 750 行"退役时单独清点"到此清点完毕：没有任何新路径读它。

### 1.3 文档
| 文件 | 改法 |
|---|---|
| `doc/opendan/Agent Memory 认知管理需求.md` TD-20（:2000-2003） | 进度行追加：2026-11-01 attention signal SQLite/CLI 已退役；"仍有价值的部分"结论按本文 §0 写明（泛指词已在 §4.5、校验不并入、回扫仍待决）。`:1432` 运行元数据行的"attention signal 管线仍在"改为已退役 |
| `doc/opendan/protocol/Agent State Protocol.md` `:26`、`:97` | 目录树去掉 `attention_signals/`；"待退役的旧调用方"只剩 `notebook_append` 与 `notebook/` |
| `doc/opendan/Agent Session SDK 实现计划.md` `:851`、`:870`、`:956`、`:1678` | 同上去掉 `attention_signals/` 与 `agent_attention_signal`；`:956`"认知由三部分组成"改为两部分 |
| `doc/llm_context/agent_tool/OpenDAN AgentTool 开发指南.md` `:292-294` | 工具名单去掉 8 个；顺手核对 `read_session_history`/`commit_session_history_improved` 是否也已不存在（Loader 重构 M0 删过） |
| `doc/llm_context/Agent Message.md` `:479` | `deny_unknown_fields` 清单去掉"`agent_attention_signal` 的 Discover* 系列" |
| `doc/arch/beta2.2-data-compatibility.md` F13（:180-188）、`:251` | 标题改"Agent Memory 与 Notebook"；删 attention signal 段与链接；版本矩阵行改 `3.0` / `0.4`（去掉 `/ 0.1`）|
| `doc/opendan/Agent元能力/Agent Memory Attention Singal Mgr.md` | 保留作历史参考（§1 已声明只作参考），文首加一行"2026-11-01 实现已删除，见 Memory 需求 TD-20" |
| `notepads/jarvis-default-goal-session-todo.md` `:335`、`:464` | 把 `agent_attention_signal.rs` 从"本次不删"清单里划掉，指向本文 |

### 1.4 不受影响（已核）
- rootfs / jarvis 的任何 `*.toml` behavior、cmd_skills 手册、websdk、Desktop：无引用。
- `opendan/src/main.rs:526-543` 只是通过 `handles_invocation_name` 分发，不直接引用，不用改。
- `agent_tool_cli_dev` 对外 `pub` 接口：只有 `handles_invocation_name` 与 `main`，无 attention 类型外泄。

## 2. 实施顺序

- [x] T1 `agent_tool`：删文件 + lib.rs 两处；`cargo check -p agent_tool`。
- [x] T2 `agent_tool_cli_dev`：按 §1.1 表删 import/常量/`TOOL_NAMES`/结构与 impl/辅助函数/注册/测试；清未用 import；`cargo test -p agent_tool_cli_dev`。
- [x] T3 `lib_opendan` 注释；`cargo check -p lib_opendan -p opendan`。
- [x] T4 文档按 §1.3 同步（8 处）。
- [x] T5（全新 DV 一项未做，见 §4.3）验收：`opendan BeginAttentionSignalExtraction` 走到 command-not-found 路径而不是被接管；`agent_tool --help`/工具清单不再出现 8 个名字；全新 DV 启动 Jarvis 整理 Goal 一轮，`<agent_root>/` 下不再生成 `attention_signals/`。
- [x] T6 提交信息引用 Memory TD-20；回写 memory 笔记。

## 3. 后续（不在本 TODO）
- Notebook 退役（TD-09）：调用方见 agent-memory-multi-session-simulation-todo §750；`rusqlite` 依赖届时才能从 `agent_tool` 去掉。
- 事后回扫是否采用（M-11，需求 §11）。

## 4. 实施记录（2026-11-01）

### 4.1 代码
- 按 §1.1 全部删除：`agent_attention_signal.rs`（2464 行）、`agent_tool/lib.rs` 两处、`agent_tool_cli_dev` 的 import/常量/`TOOL_NAMES`（24→16）/8 个 Args/Output 与 `Cli*Tool`/`validate_session_id_arg`、`session_dir` 与 5 个 attention 辅助函数/8 次注册/2 个测试；`fs_client.rs` 注释。
- 表外：`agent_tool_cli_dev` 删后不再用的 `schemars::JsonSchema`、`serde::Deserialize`、`ToolCtx`、`TypedTool` 四个 import，以及 `Cargo.toml` 的 `schemars` 依赖（`src/Cargo.lock` 在 gitignore 里）。`parse_object_bool` 的未使用告警在 HEAD 上就有（没有调用方），不属本次，未动。
- 验证：`cargo check -p agent_tool -p agent_tool_cli_dev -p libopendan -p opendan --tests` 通过，只剩改动前就有的告警；`cargo test -p agent_tool_cli_dev` 33 passed；`cargo test -p agent_tool` 280 passed / 5 ignored。

### 4.2 文档（§1.3 的 8 处 + 表外）
- 与表不同的地方：
  - AgentTool 开发指南的 `TOOL_NAMES` 名单整体改成代码里的 16 个：除 8 个 attention 工具外，`todo`、`create_workspace`、`bind_workspace`、`read_session_history`、`commit_session_history_improved` 也早已不在 `TOOL_NAMES` 里。
  - beta2.2 F13 版本矩阵写成 `4.0` / `0.4`：代码里 `agent_memory::SCHEMA_VERSION` 是 `4.0`，表里的 `3.0` 已过时；实现链接 `agent_memory.rs` 改指 `agent_memory/mod.rs`（已是目录模块）。
  - Memory 需求 :1432 一并写明 runner 字节游标已在 2026-10-23 退役（TD-19 进度已记），不再写“仍在”。
- 表外：`notepads/agent-memory-multi-session-simulation-todo.md` :42（删掉指向已删文件的链接）、:670、:750 加退役说明。
- 仓内剩下的 `attention_signal` 字样只有 TD-20 的现状/进度行、State Protocol 与 beta2.2 的“已退役”说明、旧设计文档本身。

### 4.3 验收（T5）
- `agent_tool --help` 的工具清单是 16 个，没有这 8 个名字；`agent_tool BeginAttentionSignalExtraction` / `DiscoverEvent` / `ListPendingAttentionSignals` 报 `unsupported tool`（exit 2）；`agent_tool __command_not_found__ BeginAttentionSignalExtraction` 走 command-not-found 代理返回 127。`opendan` 多入口程序以这个名字被调用时 `handles_invocation_name` 不再接管（落到 opendan 服务启动）；`STANDARD_TOOLS` 从来没有为这些名字建入口，所以 Session shell 里实际是 command not found。
- 宿主侧对照（临时 agent_root + `.meta/agent_identity.json`，`OPENDAN_AGENT_ROOT`/`OPENDAN_SESSION_ID`，经 `opendan` 多入口以 `Glob`、`read_file` 调用）：已安装的改动前 `/opt/buckyos/bin/opendan/opendan` **每次普通工具调用**都会建 `<agent_root>/attention_signals/attention_signal.sqlite`（`build_cli_tool_manager` 无条件打开 store）；新构建什么都不建。
- 顺带发现：改动前在 dev fallback（无 `OPENDAN_AGENT_ROOT`，agent_root = cwd）下也会在当前目录建 `attention_signals/`。本机留有两处这样的残留：`/opt/buckyos/data/home/devtest/Desktop/attention_signals/`（会显示在 devtest 的桌面上）与 `/tmp/attention_signals/`，按 §1.2 不自动删除，需要的话手动删。
- **未做**：全新 DV（`start.py --all`）后让 Jarvis 跑一轮整理 Goal。原因是要重装本机 DV，而 DV 上还有别的工作在跑（`buckyos-app-buckyos-systest` 容器）。按上面的对照，唯一建这个目录的代码路径已删，仓内也没有别的代码会写 `attention_signals`。

