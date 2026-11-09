# c9_c12_reason_and_clue — C9 选择理由与 C12 查找线索

Activation: decision `acted`, outcome `completed`, 1 runs, 42452 tokens

Rounds: 3

## Expected

- C9：项目 A 上次未采用服务 X，原因是当时必须离线运行；评估替代方案时先核查该限制是否仍成立。保留理由，不写成“永远不用 X”。
- C12：项目 A 的“活跃用户”有两种统计口径；比较历史数据前先查指标字典和决策记录中的适用版本。不复制 18,420 这个当前数值。

Must not appear:

- 项目 A 永远不用服务 X
- 把 18,420 当作长期事实保存

Next task: 再评估转写方案：先问离线限制是否还在；做活跃用户对比：先查口径。

## Materials

- ✓ **m1** → `absorbed` ["item_proj_a_transcription_offline_reason"]
  > 讨论项目 A 的语音转写方案时，用户说：“服务 X 不用了，现场没有外网，必须离线跑。”最终选了本地模型方案。
- ✓ **m2** → `absorbed` ["item_proj_a_active_users_comparison_clue"] — 吸收指标口径变化与查找线索；本次 Q3 数值仅留在证据中，不形成当前值认知。
  > 对比项目 A 两个季度的“活跃用户”时数字对不上：Q2 报表按 7 日登录统计，Q3 起改为 30 日有操作。两种口径都记在项目的指标字典（docs/metrics.md）和 2026-07 的决策记录里。本次 Q3 活跃用户数为 18,420。

## Resulting list

- `item_proj_a_active_users_comparison_clue@1` [active, new] basis=tool_observation explicit=false stability=contextual weight=0.7 confidence=1
  scope={"subjects":["workspace:proj-a"],"objects":["proj-a"]} tags=["项目a", "活跃用户", "统计口径", "指标字典"] last_evidence=2026-11-02T03:00:00Z
  > 项目 A 的“活跃用户”跨季度存在口径变化：Q2 按 7 日登录统计，Q3 起按 30 日有操作统计，不能直接把两者当作同一指标比较。比较历史数据前，先查项目指标字典 docs/metrics.md 和 2026-07 的决策记录，确认各期定义及后续是否又有变更。
- `item_proj_a_transcription_offline_reason@1` [active, new] basis=user_statement explicit=false stability=contextual weight=0.7 confidence=1
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "语音转写", "离线", "服务x"] last_evidence=2026-10-18T03:00:00Z
  > 项目 A 的语音转写方案未采用服务 X，是因为用户说明现场没有外网、必须离线运行，最终选择了本地模型。后续重新评估转写方案时，应先确认现场离线约束是否仍成立，而不是把服务 X 视为普遍不适用。

## Observations added

- `obs_proj_a_active_users_definitions`: 2026-11-02 对比项目 A 两个季度的“活跃用户”时数字对不上：Q2 报表按 7 日登录统计，Q3 起改为 30 日有操作。两种口径都记在项目的指标字典（docs/metrics.md）和 2026-07 的决策记录里。本次 Q3 活跃用户数为 18,420。
- `obs_proj_a_offline_choice`: 2026-10-18 讨论项目 A 的语音转写方案时，用户说：“服务 X 不用了，现场没有外网，必须离线跑。”最终选了本地模型方案。

Commit summary: 保留项目 A 转写方案选择的离线约束及原因，以及跨季度活跃用户比较时的口径核查线索；不把本次数值写入认知。

Goal result: {"decision":"acted","distance":"剩余材料 0，待复核认知 0，清理失败 0；本次没有弃置批次或被拒提交。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
