**执行清单**

**P0 直接减体积**
- [x] 删除杂质文件 [src/frame/opendan/src/.DS_Store](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/.DS_Store)。完成标准：`git diff` 里只剩删除。
- [x] 删除空壳模块 [src/frame/opendan/src/workspace/calendar.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/calendar.rs)。完成标准：`cargo check -p opendan` 通过。
- [x] 删除旧调用别名 `register_basic_workshop_tools`，修改点在 [src/frame/opendan/src/agent_environment.rs#L107](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_environment.rs#L107)。完成标准：`rg 'register_basic_workshop_tools'` 无结果。
- [x] 收敛 `workspace/todo` 中转层，先把 `crate::workspace::{TodoTool, TodoToolConfig, ...}` 改为直接从 `agent_tool` 引用，再删除 [src/frame/opendan/src/workspace/todo.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/todo.rs)。完成标准：`rg 'workspace::.*TodoTool|crate::workspace::.*TodoTool'` 无结果。
- [x] 收敛 `runtime_utils` 包装层，只保留 `find_string_pointer` 或把它迁走，再删除其余 wrapper，文件在 [src/frame/opendan/src/runtime_utils.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/runtime_utils.rs)。完成标准：`now_ms/normalize_root_path/resolve_path_from_root` 只保留一份实现。

**P1 砍掉 breaking change 下不该留的兼容层**
- [x] 移除 `exec_bash` 的 builtin fallback 逻辑，入口在 [src/frame/opendan/src/agent_bash.rs#L236](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L236)。目标：session tool env 缺失时直接报错，不再回退到进程内工具执行。
- [x] 删除 `agent-tools` 旧二进制名兼容，代码在 [src/frame/opendan/src/agent_bash.rs#L1358](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L1358)。完成标准：仓库内只认 `agent_tool`。
- [x] 删除旧环境变量别名，只保留 `OPENDAN_*`，涉及 [src/frame/opendan/src/main.rs#L87](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L87)、[src/frame/opendan/src/agent_bash.rs#L1340](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_bash.rs#L1340)、[src/frame/agent_tool/src/cli.rs#L75](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/src/cli.rs#L75)。
- [x] 删除 `main.rs` 的多级路径回退，收敛为单一来源，位置在 [src/frame/opendan/src/main.rs#L447](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/main.rs#L447)。建议优先级：CLI > `OPENDAN_*` env，去掉 spec/doc fallback。
- [x] 删除 worklog action/参数别名，只保留一套命名，位置在 [src/frame/opendan/src/worklog.rs#L301](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L301) 和 [src/frame/opendan/src/worklog.rs#L399](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L399)。
- [x] 删除 prompt 对旧消息文件名 `message_record.jsonl` 的兼容，位置在 [src/frame/opendan/src/behavior/prompt.rs#L40](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/prompt.rs#L40)。
- [x] 删除 behavior output mode 的旧别名映射，位置在 [src/frame/opendan/src/behavior/config.rs#L686](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/config.rs#L686)。
- [x] 删除 msg center 旧 box/path 名兼容，位置在 [src/frame/opendan/src/agent.rs#L1383](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent.rs#L1383)。

**P2 提取公共组件，消灭重复实现**
- [x] 把 `ai_runtime` 的 todo 查询从直写 SQL 改成调用 `agent_tool::todo` 领域层，起点在 [src/frame/opendan/src/ai_runtime.rs#L961](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L961)。
- [x] 把 `ai_runtime` 的 worklog 查询从直写 SQL 改成调用 `opendan::worklog` 服务层，起点在 [src/frame/opendan/src/ai_runtime.rs#L1087](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1087)。
- [x] 合并 workspace 记录模型，收敛 [src/frame/opendan/src/workspace/local_workspace.rs#L87](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/local_workspace.rs#L87) 和 [src/frame/agent_tool/src/workspace.rs#L19](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/src/workspace.rs#L19)。
- [x] 合并 session binding 模型，收敛 [src/frame/opendan/src/workspace/local_workspace.rs#L145](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/workspace/local_workspace.rs#L145) 和 [src/frame/agent_tool/src/workspace.rs#L27](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/src/workspace.rs#L27)。
- [x] 合并 session id/path helper，优先收敛 [src/frame/opendan/src/agent_session.rs#L1417](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_session.rs#L1417)、[src/frame/opendan/src/ai_runtime.rs#L1401](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1401)、[src/frame/opendan/src/agent.rs#L3564](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent.rs#L3564)、[src/frame/opendan/src/ai_runtime.rs#L1551](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/ai_runtime.rs#L1551)。
- [x] 删除 tool name normalization 的 provider 兼容语义，若确认外部不会再传 `module.tool`，处理点在 [src/frame/agent_tool/src/lib.rs#L277](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/src/lib.rs#L277) 和 [src/frame/agent_tool/src/lib.rs#L1679](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/src/lib.rs#L1679)。

**P3 清理双轨渲染**
- [x] 删除 DB worklog 的 legacy prompt line fallback，位置在 [src/frame/opendan/src/behavior/prompt.rs#L1359](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/prompt.rs#L1359)。
- [x] 删除 runtime worklog 的 legacy line 渲染，位置在 [src/frame/opendan/src/behavior/prompt.rs#L1648](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/behavior/prompt.rs#L1648)。
- [x] 如果保留结构化渲染，检查 [src/frame/opendan/src/worklog.rs#L564](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worklog.rs#L564) 是否仍需要对旧字段做兜底。

**建议执行顺序**
1. 先做 P0，风险最低，能快速减小代码量。
2. 再做 P1，这是最符合 breaking change 预期的一轮。
3. 然后做 P2，把重复查询和模型合并。
4. 最后做 P3，收口提示词和 worklog 渲染。

**每轮验收**
- `cd /Users/liuzhicong/project/buckyos/src && cargo check -p agent_tool -p opendan`
- 对兼容层删除项额外跑一次 `rg`，确认旧名字、旧 env、旧 action 已经清空
- 对 `exec_bash` 和 todo/worklog 改动，至少跑对应模块测试
