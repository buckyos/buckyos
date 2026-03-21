
这次 review 的核心判断是：`opendan` 目前最大可压缩空间不在核心 `agent/todo/worklog` 逻辑本身，而在“多代兼容层 + 多入口包装 + 重复 helper”。我做了静态梳理，并跑了 `cargo check -p opendan --all-targets` 与 `cargo test -p opendan -- --list`；当前还有直接可见的残留信号，比如 [agent_environment.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L27) 的未使用 import，和 [behavior/prompt.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/prompt.rs#L1512)、[behavior/prompt.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/prompt.rs#L2004) 的未使用变量。

先说最重要的三个 findings。

1. Session 存储已经变成“双实现”，但生产写路径实际上只有 file 版。当前 `AgentSessionMgr` 只写 `session.json`/`summary.md`，见 [agent_session.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_session.rs#L948) 和 [agent_session.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_session.rs#L1233)。但 `AiRuntime` 仍保留 SQLite 读/改分支，见 [ai_runtime.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1345)、[ai_runtime.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1498)、[ai_runtime.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1559)、[ai_runtime.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1645)；而 sqlite 写入只出现在测试 helper，见 [ai_runtime.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L2351)。这是最高优先级删减点。

2. Tool CLI 暴露面被拆成了 1 个真实实现 + 一组完全相同的 wrapper bin。Cargo 里声明了大量 bin，见 [Cargo.toml](/Users/liuzhicong/project/buckyos/src/frame/opendan/Cargo.toml#L39)。这些 wrapper 文件本质上都只是调用 `run_process()`，例如 [agent_tool_bin.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_tool_bin.rs#L1)、[agent_tools_bin.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_tools_bin.rs#L1)、[read_file_bin.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/read_file_bin.rs#L1)。同时 `agent_bash` 还在搜索 legacy 名 `agent-tools`，见 [agent_bash.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L1358)。而安装配置里真正定义的模块是 `agent_tool`，不是 `agent-tools`，见 [bucky_project.yaml](/Users/liuzhicong/project/buckyos/src/bucky_project.yaml#L46) 和 [bucky_project.yaml](/Users/liuzhicong/project/buckyos/src/bucky_project.yaml#L87)。

3. breaking change 已经成立，但启动/配置/schema 仍保留大量历史兼容入口。启动参数/环境变量/agent spec/doc 多源解析集中在 [main.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L83) 和 [main.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L455)。行为配置还接受多个旧字段别名，见 [behavior/config.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/config.rs#L54)、[behavior/config.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/config.rs#L315)、[behavior/config.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/config.rs#L424)。workspace/worklog 也有兼容桥接，见 [agent_environment.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L1235)、[workspace/workshop.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/workshop.rs#L357)、[worklog.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L1833)。

**1）可以删除的组件**
- [src/frame/opendan/src/.DS_Store](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/.DS_Store) 是纯杂质文件，直接删。
- [src/frame/opendan/src/workspace/calendar.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/calendar.rs#L1) 只有注释，且 [workspace/mod.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/mod.rs#L1) 没有引入它，属于未编译残留。
- `agent-tools` 旧别名及其 wrapper：[agent_tools_bin.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_tools_bin.rs#L1)。如果确认 breaking change，`agent_bash` 里的 legacy 查找也一起删，见 [agent_bash.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L1376)。
- 除 `agent_tool` 以外的各个同构 wrapper bin：`read_file/write_file/edit_file/get_session/todo/create_workspace/bind_workspace/check_task/cancel_task`。建议改成“只保留 `agent_tool` + 安装时生成 symlink/argv0 分发”。
- `AiRuntime` 的 SQLite session 分支。因为当前生产写路径不是 sqlite，这一整套更像未完成迁移，而不是有效实现。
- [agent_environment.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L107) 的 `register_basic_workshop_tools`，只是旧调用名别名。
- [worklog.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L1833) 的 `legacy_log_view` 以及返回里的 `log/logs` 镜像字段；现有 `record/records` 已足够。

**2）发现重复实现，下一步可以提取公共组件**
- 路径处理重复最明显：`normalize_abs_path` / `resolve_path_in_agent_env` / `normalize_agent_env_root` 同时散落在 [buildin_tool.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/buildin_tool.rs#L1113)、[workspace/workshop.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/workshop.rs#L1201)、[workspace/local_workspace.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/local_workspace.rs#L1099)。建议收敛成一个 `path_utils`。
- JSON 参数读取重复：`require_string`、`optional_string`、`optional_u64`、`u64_to_usize`、`now_ms` 在 `ai_runtime/worklog/todo/buildin_tool/local_workspace` 多处重复。建议抽成统一的 `json_args`/`runtime_utils`。
- CLI 入口重复：11 个 bin 文件同构，应该提成单入口。
- 启动环境解析重复：`main.rs`、[agent_tools_cli.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_tools_cli.rs#L65)、`agent_bash.rs` 都在维护一套近似的 env alias 语义，应该收敛成一个 typed launch/runtime context。
- workspace 标识解析重复且弱类型：见 [agent_environment.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L1235) 和 [workspace_path.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace_path.rs)。建议引入强类型 `WorkspaceBindingRef`。

**3）兼容性处理，可以直接清掉**
- 启动 env alias：`AGENT_ID/AGENT_INSTANCE_ID`、`AGENT_ENV/AGENT_ROOT`、`AGENT_BIN/AGENT_PACKAGE_ROOT`、`SERVICE_PORT/LISTEN_PORT`，见 [main.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L83)。
- agent spec/doc 的多级 JSON pointer 回退，见 [main.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L474) 和 [main.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L516)。breaking change 后应只保留一套字段。
- `agent-tools` 旧二进制名兼容，见 [agent_bash.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L1376)。
- 行为配置旧字段 alias：`fallto`、`failed_back`、`fallback_behavior`、`total_limt`、`session_summary`、`allow_tools`，见 [behavior/config.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/config.rs#L54) 等。
- `tools.json` 中对 `worklog_manage` 的兼容解析，见 [workspace/workshop.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/workshop.rs#L357)。如果只为 write-audit 配置，应改成独立字段，不要借旧 tool 名承载。
- worklog 返回里的 `log/logs` 兼容镜像，见 [worklog.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L398)。
- `workspace_info` 的弱类型多路径兼容读取，见 [agent_environment.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L1235)。

如果按“尽快缩代码规模”的收益排序，我建议顺序是：
1. 删 `AiRuntime` 的 session SQLite 分支，统一只保留 file 存储。
2. 收缩到单一 `agent_tool` 二进制，删除 alias bin 和 `agent-tools` 兼容。
3. 砍掉 `main.rs`/behavior/worklog/workspace 的历史字段 alias。
4. 抽公共 path/json helper，再顺手清掉未使用变量和 import。

