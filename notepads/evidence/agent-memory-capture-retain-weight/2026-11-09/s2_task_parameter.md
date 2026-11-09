# s2_task_parameter — S2 一次中文发帖不是长期语言偏好

Activation: decision `acted`, outcome `completed`, 1 runs, 40826 tokens

Rounds: 3

## Expected

- 不新增长期认知：“用中文发这一条”只是本次任务要求。
- 旧历史条目 item_agent_posted_twice 没有复用理由，退出有效列表或保持不动；不得被改写成“用户偏好中文”。

Must not appear:

- 用户长期偏好中文
- 曾经发过帖等于本次已完成

Next task: 新的发帖任务按当次要求执行，不默认长期中文，也不把曾经发布当作本次已完成。

## Materials

- ✓ **m1** → `discarded` — 用户仅要求本次发问候及中文自我介绍，发布完成属于任务状态；没有未来适用的偏好、约定或可复用经验。
  > 用户说：“去 Home Station 发个问候，再用中文发一条自我介绍。”两条都已发布。

## Resulting list

- `item_agent_posted_twice@2` [stale, revised] basis=inference explicit=false stability=contextual weight=0.6 confidence=0.7
  scope={"subjects":["did:bns:jarvis.alice"],"objects":["home-station"]} tags=["home station", "发帖"] last_evidence=2026-10-28T10:00:00Z
  > 已在 Home Station 发过两条帖子（问候、中文自我介绍），帖子未修改，不是定期发帖。

## Observations added


Commit summary: 丢弃一次性发帖要求及完成记录，并将已有的纯发帖历史记忆标记为 stale；不推断长期语言或发帖偏好。

Goal result: {"decision":"acted","distance":"新材料、到期待处理材料、清理重试和待复核认知均为 0。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
