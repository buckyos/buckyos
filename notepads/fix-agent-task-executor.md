
**主要差距**
- **P1 route 阻塞闭环缺失**：`task_route` session 是 `bind_task: false` 创建的，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:260)，所以它进入 `WaitingInput` 时不会创建 `human.input` 子任务；`feedback_task_waiting_for_input` 需要 `task_binding` 才会继续，[agent_session.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_session.rs:4394)。这和文档中“路由不确定时创建 human.input 并挂起 root task”的要求不一致。


要修

- **P1 幂等绑定不够可靠**：幂等判断只看 `data.agent_delegate.execution.session_id`，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:168)。但 direct path 先创建 session，之后才写 `execution.session_id`，[agent.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent.rs:1701)。如果 session 已落盘但 TaskMgr update 失败/进程崩溃，下一次 sweep 会重新创建第二个 WorkSession，破坏文档里的 “task_id 只能接收一次”。

要修

- **P1 human.input 恢复会丢新 response**：`resume_waiting_delegate_task` 在无 `execution.session_id` 时把 response patch 到 TaskMgr 后返回 true，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:380)，但 `process_agent_delegate_task` 继续使用旧的 `task.data` 做 direct/task_route 判断，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:168)。也就是说路由阶段补充的人类输入不会进入下一次 route objective。

这个原理上，是worksession恢复后，通过task工具读取到human.input 来推进的

- **P2 task_route 的输出/复用路径没落地**：文档要求 task_route 输出 resolved/need_human_input，并可复用已有 worksession。当前只记录 `route.session_id`，没有消费 route session 的结构化结果，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:279)。Jarvis 的 `task_route` behavior 还白名单了不存在/拼错的 `disptach_task`，[task_route.toml](/Users/liuzhicong/project/buckyos/src/rootfs/bin/buckyos_jarvis/behaviors/task_route.toml:37)，实际工具名里没有它，[worksession_tools.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/worksession_tools.rs:67)。

这个步骤worksession没启动，比较麻烦。可以用“fail first的原则“，直接把task设置为失败，让上层创建新的task

- **P2 事件订阅只做了 runner inbox，没有执行中 root/task channel**：当前只订阅 `/task_mgr/runner/{runner}/task_ready`，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:50)。TaskMgr 已发布 per-task/root change event，[server.rs](/Users/liuzhicong/project/buckyos/src/kernel/task_manager/src/server.rs:278)，但 AgentRuntime 没订阅执行中 root channel，所以 cancel、child completed 等主要靠 5s poll 兜底。

- **P2 TaskCenter 侧还没接入真实 schema**：桌面 TaskCenter 目前仍用 `TaskCenterMockStore`，[TaskCenterRoute.tsx](/Users/liuzhicong/project/buckyos/src/frame/desktop/src/app/task-center/TaskCenterRoute.tsx:44)，代码里没有 `agent.delegate` / `human.input` 结构化渲染或 response 写回入口。这和文档里的“TaskCenter 首页优先展示 WaitingForApproval 子任务”还有明显距离。

UI 先不管，等系统的task.data schema都完善后统一更新UI

- **P3 runner 映射还偏本地实现**：默认 runner 是 sanitized `agent_id()`，[agent_task_executor.rs](/Users/liuzhicong/project/buckyos/src/frame/opendan/src/agent_task_executor.rs:18)，而文档提醒当前 TaskMgr runner 语义仍偏 `node_id/app_id`，需要明确 AgentRuntime ID 到 app/runtime id 的落地映射。

目前每个agent-runtime启动的时候都有一个appid (包含到了app的owner userid)，要确认agent task executor用争取的app_id去拿等待自己执行的任务。



