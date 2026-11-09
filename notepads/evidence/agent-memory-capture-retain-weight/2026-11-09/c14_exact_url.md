# c14_exact_url — C14 精确入口：完整 URL 及用途可恢复，本次参数不入列表

Activation: decision `acted`, outcome `completed`, 1 runs, 57375 tokens

Rounds: 4
- refused: "the plan was refused: invalid argument: tag has forbidden character '_': \"test_endpoint\""

## Expected

- 一条使用线索：用户所说的项目 A“测试入口”指其指定的测试服务，不是生产；完整 URL 从项目配置 test_endpoint 读取（认知中带完整 URL 也可接受，但必须完整：端口 8443、路径 /api/v2、参数 tenant=qa 都在，或都不写而指向配置）。观察里保留完整原值。
- m2 的电话号码只是本次任务参数：丢弃，不进入长期认知。

Must not appear:

- 凭摘要补全或截短 URL
- 把一次性的电话号码存成长期认知

Next task: 用户说“测试入口”时定位到测试服务并从配置读取当前地址，不凭摘要重建 URL。

## Materials

- ✓ **m1** → `absorbed` ["item_proj_a_test_endpoint_convention"] — 明确面向以后使用的项目称谓与环境区分约定；精确地址留在证据中，当前值查项目配置。
  > 用户说：“项目 A 的测试入口是 https://staging.proj-a.example.com:8443/api/v2?tenant=qa ，别和生产搞混；以后我说‘测试入口’就是它。”该地址也写在项目配置的 test_endpoint 项里。
- ✓ **m2** → `discarded` — 号码仅是这次回调测试的任务参数，执行记录不构成长期偏好、联系信息或可复用经验。
  > 用户说：“这次回调先打到 13800001234 这个号码上试一下。”已按此执行。

## Resulting list

- `item_proj_a_test_endpoint_convention@1` [active, new] basis=user_statement explicit=true stability=contextual weight=0.85 confidence=1
  scope={"subjects":["did:bns:alice"],"objects":["proj-a"]} tags=["项目a", "测试入口", "环境"] last_evidence=2026-10-26T03:00:00Z
  > 用户约定在项目 A 中，“测试入口”指其指定的测试端点，不是生产入口。后续涉及该称谓时，应到项目配置的 test_endpoint 项查取当前地址，避免混用环境。

## Observations added

- `obs_proj_a_test_endpoint`: 2026-10-26T03:00:00Z，用户说：“项目 A 的测试入口是 https://staging.proj-a.example.com:8443/api/v2?tenant=qa ，别和生产搞混；以后我说‘测试入口’就是它。”该地址也写在项目配置的 test_endpoint 项里。

Commit summary: 保留项目 A 的“测试入口”称谓及环境区分约定，以配置项作为当前地址的查找位置；丢弃仅供一次回调测试使用的号码。

Goal result: {"decision":"acted","distance":"新材料、到期待处理材料、清理失败与待复核认知均为 0。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
