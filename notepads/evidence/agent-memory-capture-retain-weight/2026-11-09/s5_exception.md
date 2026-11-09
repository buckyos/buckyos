# s5_exception — S5 持续要求与本次例外；本批没提到的旧认知保持原样

Activation: decision `acted`, outcome `completed`, 1 runs, 41758 tokens

Rounds: 3

## Expected

- 一页以内的偏好保持有效：m1 是本次例外，不修订、不降权、不撤回。
- m2 是同义重申：判重，或作为新证据补到原条目（last_evidence_at 可更新），不新建条目，weight 不因此机械上调。
- 本批没提到的 item_owner_proj_a_deliver_source_and_pdf 保持原样。

Must not appear:

- 本次例外撤回或削弱一页以内的偏好
- 因为本批没提到而改动交付形式的约定
- 重申一次就机械上调 weight

Next task: 下一期项目 A 报告仍默认一页以内。

## Materials

- ✓ **m1** → `duplicate` ["item_owner_proj_a_report_one_page"] — “这次详细点”仅为本期报告的任务例外，不撤销项目 A 后续报告一页以内的长期要求；不修订原偏好或降低权重，交付进度不记入认知。
  > 用户在项目 A 的本期报告任务里说：“这次详细点。”已按详细版交付。
- ✓ **m2** → `duplicate` ["item_owner_proj_a_report_one_page"] — 用户重申同一项目、同一范围的一页篇幅要求，未增加条件或改变原偏好，无需新增或修订认知。
  > 用户再次提到：“项目 A 的报告还是一页以内。”

## Resulting list

- `item_owner_proj_a_deliver_source_and_pdf@1` [active, as seeded] basis=user_statement explicit=true stability=stable weight=0.7 confidence=0.9
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "交付形式"] last_evidence=2026-10-12T02:00:00Z
  > 用户要求项目 A 的后续交付同时提供可编辑源文件和 PDF。
- `item_owner_proj_a_report_one_page@1` [active, as seeded] basis=user_statement explicit=true stability=stable weight=0.7 confidence=0.9
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "报告", "篇幅"] last_evidence=2026-10-20T02:00:00Z
  > 用户要求项目 A 的后续报告控制在一页以内。

## Observations added


Commit summary: 两条材料均归于既有的项目 A 报告篇幅偏好：一次任务例外和一次重申；保留原偏好及权重，其他交付要求不变。

Goal result: {"decision":"acted","distance":"待处理材料 0，待清理 0，到期认知复核 0；无放弃批次或被拒提交。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
