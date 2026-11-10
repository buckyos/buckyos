# Agent skill PIKG 样例

本工程演示独立的 `app_type=agent_skills` 包，不是普通 App 或 Agent 模板。元数据位于 `dapp_meta/`，唯一子包 `agent_skills` 读取 `skills/`：

```text
skills/cmd_skills/hello/SKILL.md        work skill
skills/tools/hello/bin/hello            它自带的命令
skills/plan_skills/hello_plan/SKILL.md  plan skill（不写命令）
```

在本目录使用已安装的项目 CLI，离线构建和校验：

```bash
npx buckyos pikg doctor ./dapp_meta
npx buckyos pikg build ./dapp_meta
npx buckyos pikg pack ./dapp_dist
npx buckyos pikg info ./dapp_dist/hello-skills-0.1.0.pikg
```

`dapp_dist/` 是生成物，改了 `skills/` 之后用上面的命令重新生成。

安装只有 owner 能做，需要已配置目标 Zone、已运行的 Agent 和该 Agent 接受的 owner 身份；将 `<agent_id>` 换成实际 AgentId。源目录可以直接装来试用，也可以装打好的包：

```bash
npx buckyos skill install ./skills --agent <agent_id>
npx buckyos skill install ./dapp_dist/hello-skills-0.1.0.pikg --agent <agent_id>
npx buckyos skill list --agent <agent_id>
npx buckyos skill list --agent <agent_id> --kind plan
```

该路径调用 Agent 的 `skills.*` kRPC（先给出安装计划与风险清单，确认后写入），不调用普通 `app install`。Agent 自己没有安装命令，`agent-session skill` 只有 `list` / `status` / `revise`。已有 Session 的冻结手册不会因安装而替换，新 Session 才读取新版本。

扩展时可在 `skills/` 中加入 `goal_skills/<name>/SKILL.md`、`cmd_skills/<name>/references/` 或 `tools/<name>/install.toml`。写法、lint 与发行见 [skill 开发快速上手](../../doc/sdk/skill-dev-quickstart.md)，机制见 [OpenDAN 开发指南](../../doc/sdk/OpenDAN_Agent_Dev_Guide.md) §2.4，PIKG 侧见 [PIKG 模块 §6.6](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md#7-agent-skill-包)；其他资料见 [PIKG 文档总览](../../doc/pikg/README.md)。
