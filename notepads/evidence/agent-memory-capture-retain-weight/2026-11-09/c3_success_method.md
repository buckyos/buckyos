# c3_success_method — C3 成功方法：保留有验证支持的检查经验及边界

Activation: decision `acted`, outcome `completed`, 1 runs, 42225 tokens

Rounds: 3

## Expected

- 一条成功方法经验：用转换器 v2 批量处理项目 A 的模板 T 月报前，先用含表格和页眉的代表样本检查布局，可及早发现模板问题。basis=tool_observation（Agent 亲自执行并看到结果），范围限转换器 v2 与模板 T 月报，换版复核；不写成保证成功，不复制完整批处理步骤。

Must not appear:

- 样本检查通过即可保证整批通过
- 只记失败不记有效方法
- 把完整批处理步骤抄进认知

Next task: 下一批模板 T 月报转换前先做样本检查；完整步骤查 Skill。

## Materials

- ✓ **m1** → `absorbed` ["item_proj_a_t_v2_sample_check"] — 提供先试转样本发现页眉问题、修正映射后整批通过的独立经验。
  > 用转换器 v2 批量转换项目 A 的模板 T 月报（10 月批次）前，先挑了一份含表格和页眉的样本试转，发现页眉错位；修正模板映射后整批 42 份通过布局检查。
- ✓ **m2** → `absorbed` ["item_proj_a_t_v2_sample_check"] — 提供下一批次独立验证，并补充表格跨页与分页参数的检查重点；与首批经验合并为同一适用范围的认知。
  > 11 月批次同样先试转含表格和页眉的样本：这次发现表格跨页被截断，调整分页参数后整批 40 份通过。两次都是样本检查先暴露了模板问题。

## Resulting list

- `item_proj_a_t_v2_sample_check@1` [active, new] basis=tool_observation explicit=false stability=contextual weight=0.7 confidence=0.98
  scope={"subjects":["workspace:proj-a"],"objects":["proj-a"]} tags=["转换器", "模板t", "月报", "样本检查"] last_evidence=2026-11-05T03:00:00Z
  > 用转换器 v2 批量转换项目 A 的模板 T 月报前，先试转含表格和页眉的代表性样本，重点检查页眉布局和表格跨页；样本检查曾在两个批次中提前暴露问题，分别修正模板映射、调整分页参数后整批通过。此经验限于该模板与转换器版本，模板或转换器变更时需重新验证。

## Observations added

- `obs_proj_a_t_v2_nov_sample`: 2026-11-05：11 月批次同样先试转含表格和页眉的样本，这次发现表格跨页被截断；调整分页参数后整批 40 份通过。记录指出两次都是样本检查先暴露了模板问题。
- `obs_proj_a_t_v2_oct_sample`: 2026-10-15：用转换器 v2 批量转换项目 A 的模板 T 月报（10 月批次）前，先挑了一份含表格和页眉的样本试转，发现页眉错位；修正模板映射后整批 42 份通过布局检查。

Commit summary: 合并两次独立批次的验证经验，保留项目 A 模板 T 月报在转换器 v2 下先做代表性样本检查的适用条件与检查重点。

Goal result: {"decision":"acted","distance":"剩余材料 0，待复核认知 0；本批新增 1 条可复用经验，无放弃批次或被拒提交。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
