# guidance_user_method — 用户提供的方法先是线索，验证后才是技术经验；持续指导形成有范围的约定

Activation: decision `acted`, outcome `completed`, 1 runs, 42362 tokens

Rounds: 3

## Expected

- m1：用户提供的方法线索，未验证。若保留，必须表述为用户的说法/建议（basis=user_statement 表示“用户这么说”，或 third_party/低置信），不能写成已验证的工具规律；暂缓等待验证也可接受。
- m2：限项目 A 周报的持续偏好，explicit=true，不扩大到所有报告。
- m3：本次任务要求，丢弃；不形成“默认先给计划”。

Must not appear:

- tool-y 先建缓存目录是已验证的规律
- “今天先给计划”变成默认先给计划
- 周报结构要求扩大到所有报告

Next task: 跑 tool-y 时把建缓存目录当作待验证线索；项目 A 周报默认先给结论与待决事项。

## Materials

- ✓ **m1** → `absorbed` ["item_proj_a_tool_y_cache_suggestion"] — 作为用户提出但未实测的方法保留，不推断技术效果。
  > 用户说：“tool-y 先建好缓存目录就能跑。”我还没有试。
- ✓ **m2** → `absorbed` ["item_alice_proj_a_weekly_structure"] — 明确适用于以后项目 A 周报的长期要求。
  > 用户说：“以后项目 A 的周报先给结论和待决事项，明细放附件。”
- ✓ **m3** → `discarded` — “今天这份”限定为该次任务先看计划的要求，不能推广为长期偏好；留在任务记录而非记忆。
  > 用户说：“今天这份先给我看计划再动手。”

## Resulting list

- `item_alice_proj_a_weekly_structure@1` [active, new] basis=user_statement explicit=true stability=stable weight=0.8 confidence=1
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "周报", "结构"] last_evidence=2026-11-01T04:00:00Z
  > 用户要求以后项目 A 的周报先给结论和待决事项，明细放附件；此要求限于项目 A 的周报。
- `item_proj_a_tool_y_cache_suggestion@1` [active, new] basis=user_statement explicit=false stability=contextual weight=0.5 confidence=1
  scope={"subjects":["workspace:proj-a"],"objects":["tool-y"]} tags=["tool-y", "缓存目录"] last_evidence=2026-11-01T03:00:00Z
  > 在项目 A 使用 tool-y 时，可考虑用户提出的先建好缓存目录的方法；该方法尚未经代理实测，不能当作已验证的运行条件或修复办法。

## Observations added

- `obs_alice_proj_a_weekly_structure`: 2026-11-01T04:00:00Z，用户说：“以后项目 A 的周报先给结论和待决事项，明细放附件。”
- `obs_proj_a_tool_y_user_method`: 2026-11-01T03:00:00Z，代理记录用户说：“tool-y 先建好缓存目录就能跑。”代理明确表示还没有试。

Commit summary: 保留项目 A 周报的长期结构要求，以及 tool-y 缓存目录的用户建议（未验证）；丢弃仅针对当日任务的先看计划要求。

Goal result: {"decision":"acted","distance":"3 条材料全部处置，新增 2 条认知；无剩余材料、清理失败或到期复核。tool-y 方法未验证，已在认知中保留该限制。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
