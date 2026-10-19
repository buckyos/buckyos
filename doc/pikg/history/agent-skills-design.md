# 旧 Skills Mgr：历史设计摘要

> 历史参考。原 `Agent Skill.md` 已在 2026-10-17 标明废弃；旧 `skills/skill.yaml/SKILL.md` 与 `agent-skills` CLI 不作为当前 Agent 扩展接口。

旧设计把 skill 作为 procedural memory artifact，规划 Compiler、Registry、Recall、Verification、Curator、Governance、排名、降权、归档和回滚；它与 Self-Improve candidate/proposal 的配合只保留作研究背景，不表示这些模块已完整交付。

当前 cmd_skills/goal_skills、可选 tools、Session 冻结与安装规则只在 [OpenDAN 开发指南](../../sdk/OpenDAN_Agent_Dev_Guide.md) 维护；独立 PIKG 的构造与消费见 [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) 和 [skill 样例](../../../test/skills_pikg_sample/README.md)。

## 原始稿追溯

- [doc/opendan/Agent元能力/Agent Skill.md](https://github.com/buckyos/buckyos/blob/e8543cc9aaf288c0d4d9dadefc0a66bade637c18/doc/opendan/Agent%E5%85%83%E8%83%BD%E5%8A%9B/Agent%20Skill.md)
