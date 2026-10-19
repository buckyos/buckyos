# Agent skill PIKG 样例

本工程演示独立的 `app_type=agent_skills` 包，不是普通 App 或 Agent 模板。元数据位于 `dapp_meta/`，唯一子包 `agent_skills` 读取 `skills/`；示例手册在 `skills/cmd_skills/hello.md`。

在本目录使用已安装的项目 CLI，离线构建和校验：

```bash
npx buckyos pikg doctor ./dapp_meta
npx buckyos pikg build ./dapp_meta
npx buckyos pikg pack ./dapp_dist
npx buckyos pikg info ./dapp_dist/hello-skills-0.1.0.pikg
```

`dapp_dist/` 是生成物。需要已配置目标 Zone、已运行的 Agent 和该 Agent 接受的身份，才能执行安装与查询；将 `<agent_id>` 换成实际 AgentId：

```bash
npx buckyos skill install ./dapp_dist/hello-skills-0.1.0.pikg --agent <agent_id>
npx buckyos skill list --agent <agent_id>
```

该路径调用 Agent 的 `skills.install/list`，不调用普通 `app install`。Agent 内部也可使用 `agent-session skill install`；已有 Session 的冻结手册不会因安装而替换，新 Session 才读取新版本。

扩展时可在 `skills/` 中加入 `goal_skills/` 或 `tools/<cmd_skill>/bin/`。格式、覆盖与删除语义见 [OpenDAN 开发指南](../../doc/sdk/OpenDAN_Agent_Dev_Guide.md)、[PIKG 模块 §6.6](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md#7-agent-skill-包)；其他资料见 [PIKG 文档总览](../../doc/pikg/README.md)。
