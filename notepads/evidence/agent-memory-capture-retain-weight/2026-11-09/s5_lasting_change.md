# s5_lasting_change — S5 明确的长期修改修订旧偏好；类型例外只补例外

Activation: decision `acted`, outcome `completed`, 1 runs, 41397 tokens

Rounds: 3

## Expected

- 修订同一条偏好并补充例外：项目 A 的报告一页以内，技术评审报告除外。不新建矛盾条目，不抹掉其他报告类型的要求，explicit 保持 true。

Must not appear:

- 整条一页以内的要求被撤回
- 新旧两条矛盾结论同时有效

Next task: 技术评审报告可超过一页；周报等仍一页以内。

## Materials

- ✓ **m1** → `absorbed` ["item_owner_proj_a_report_one_page"] — 这是对已有篇幅偏好的长期范围修订，而非单次任务例外。
  > 用户说：“项目 A 的技术评审报告以后不受一页限制，其他报告照旧。”

## Resulting list

- `item_owner_proj_a_report_one_page@2` [active, revised] basis=user_statement explicit=true stability=stable weight=0.7 confidence=1
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "报告", "篇幅", "技术评审"] last_evidence=2026-11-06T02:00:00Z
  > 用户要求项目 A 的后续报告控制在一页以内，但技术评审报告以后不受一页限制。

## Observations added

- `obs_owner_proj_a_technical_review_exception`: 用户说：“项目 A 的技术评审报告以后不受一页限制，其他报告照旧。”该表述发生于 2026-11-06T02:00:00Z，由代理转述。

Commit summary: 修订项目 A 报告篇幅偏好：技术评审报告长期豁免一页限制，其他报告仍控制在一页以内。

Goal result: {"decision":"acted","distance":"材料已全部处理，无积压、待清理项或到期认知复核。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
